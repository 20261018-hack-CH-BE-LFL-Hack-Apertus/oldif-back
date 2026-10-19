use crate::{
    error::{Error, Result},
    infrastructure::store::{Record, Write},
    state::App,
};
use serde_json::{Value, json};
use std::collections::HashMap;

const DATASET: &str = "stapfer-1799-answers";
const MAX_ITEMS: usize = 200;
const MAX_BYTES: usize = 3_000_000;

pub async fn seed(app: &App) -> Result<()> {
    match app.store.get("public", "dataset", DATASET).await {
        Ok(_) => return Ok(()),
        Err(error) if error.0 == axum::http::StatusCode::NOT_FOUND => {}
        Err(error) => return Err(error),
    }
    let rows = include_str!("../../data/stapfer-answer-sources.jsonl")
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<Vec<Value>, _>>()?;
    let manifest: Value = serde_json::from_str(include_str!("../../data/stapfer-import-manifest.json"))?;
    let existing = app.store.list("public", "source").await?;
    let pending = missing(&rows, &existing)?;
    for chunk in chunks(pending)? {
        if let Err(error) = app.store.batch(chunk.clone()).await {
            if error.0 != axum::http::StatusCode::CONFLICT {
                return Err(error);
            }
            // Another backend may have committed this chunk during a rolling deployment.
            let existing = app.store.list("public", "source").await?;
            let pending = missing(&chunk.into_iter().map(|w| w.data).collect::<Vec<_>>(), &existing)?;
            if !pending.is_empty() {
                app.store.batch(pending).await?;
            }
        }
    }
    let dataset = json!({"name":"Stapfer-Enquête 1799 · Question-answer export","title":"Stapfer-Enquête 1799 · Question-answer export",
        "description":"2245 transcription groups from the second supplied workbook; question rows, numeric values and missing answers retain separate provenance. This collection overlaps the full-text export.",
        "record_count":rows.len(),"accepted_answer_rows":manifest["files"][1]["accepted_rows"],"missing_answer_rows":manifest["files"][1]["rejected_rows"],
        "licence":"terms_open","source_url":"https://www.stapferenquete.ch/","kind":"published-question-answer-groups","page_citations":false,
        "provenance":"supplied-challenge-workbook","source_sha256":manifest["files"][1]["sha256"],"import_manifest":manifest,
        "rejected_rows":include_str!("../../data/stapfer-rejected-rows.jsonl").lines().map(serde_json::from_str::<Value>).collect::<std::result::Result<Vec<_>,_>>()?});
    match app.store.put(Write::new("public", "dataset", DATASET, dataset)).await {
        Ok(_) => Ok(()),
        Err(error) if error.0 == axum::http::StatusCode::CONFLICT => {
            app.store.get("public", "dataset", DATASET).await?;
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn missing(rows: &[Value], existing: &[Record]) -> Result<Vec<Write>> {
    let existing = existing.iter().map(|r| (r.id.as_str(), &r.data)).collect::<HashMap<_, _>>();
    let mut pending = Vec::new();
    for row in rows {
        let id = row["id"].as_str().ok_or_else(|| Error::bad("Bundled source has no identifier"))?;
        if let Some(prior) = existing.get(id) {
            if prior["sha256"] != row["sha256"] || prior["text_sha256"] != row["text_sha256"]
                || prior["dataset_id"] != row["dataset_id"] || prior["source_sha256"] != row["source_sha256"] {
                return Err(Error::bad("Immutable bundled source differs from the resumed import"));
            }
        } else {
            pending.push(Write::new("public", "source", id, row.clone()));
        }
    }
    Ok(pending)
}

fn chunks(writes: Vec<Write>) -> Result<Vec<Vec<Write>>> {
    let mut result = vec![];
    let mut chunk = vec![];
    let mut bytes = 32;
    for write in writes {
        let size = serde_json::to_vec(&write)?.len() + 1;
        if size > MAX_BYTES - 32 {
            return Err(Error::bad("Bundled source exceeds the seed request limit"));
        }
        if chunk.len() == MAX_ITEMS || bytes + size > MAX_BYTES {
            result.push(std::mem::take(&mut chunk));
            bytes = 32;
        }
        bytes += size;
        chunk.push(write);
    }
    if !chunk.is_empty() { result.push(chunk); }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seed_requests_are_bounded_and_resume_preserves_immutable_sources() {
        let rows = include_str!("../../data/stapfer-answer-sources.jsonl").lines().map(|l|serde_json::from_str(l).unwrap()).collect::<Vec<Value>>();
        assert_eq!(rows.len(), 2245);
        let first = Record { owner:"public".into(),kind:"source".into(),id:rows[0]["id"].as_str().unwrap().into(),version:1,data:rows[0].clone(),created_at:0 };
        let pending = missing(&rows, &[first.clone()]).unwrap();
        assert_eq!(pending.len(), 2244);
        let batches = chunks(pending).unwrap();
        assert!(batches.len() < 20);
        for batch in batches {
            assert!(batch.len() <= MAX_ITEMS);
            assert!(serde_json::to_vec(&json!({"items":batch})).unwrap().len() <= MAX_BYTES);
        }
        let mut corrupt = first;
        corrupt.data["text_sha256"] = json!("changed");
        assert!(missing(&rows, &[corrupt]).is_err());
    }
}
