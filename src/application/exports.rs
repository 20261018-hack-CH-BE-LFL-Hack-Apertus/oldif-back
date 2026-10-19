use crate::{
    application::{auth::hash, sources},
    domain::tabular_cell,
    error::{Error, Result},
    infrastructure::store::Write,
    state::App,
};
use serde_json::{Value, json};
use uuid::Uuid;

pub async fn review(app: &App, owner: &str, run_id: &str, input: Value) -> Result<Value> {
    let run = app.store.get(owner, "run", run_id).await?;
    let decision = input["decision"].as_str().unwrap_or("");
    let reason = input["reason"].as_str().unwrap_or("").trim();
    if !["accepted", "rejected"].contains(&decision) || reason.is_empty() || reason.len() > 4000 {
        return Err(Error::bad("A review decision and reason are required"));
    }
    if input["expected_version"].as_i64() != Some(run.version) {
        return Err(Error(
            axum::http::StatusCode::CONFLICT,
            "Reload the current run revision before reviewing".into(),
        ));
    }
    if run.data["status"] != "complete" {
        return Err(Error::bad("Only completed runs can be reviewed"));
    }
    let mut data = run.data.clone();
    data["review"] = json!({"decision":decision,"reason":reason,"actor":owner});
    let review_id = Uuid::new_v4().to_string();
    let records=app.store.batch(vec![Write::update(&run,data),Write::new(owner,"review",&review_id,json!({"run_id":run_id,"decision":decision,"reason":reason,"actor":owner,"previous_version":run.version,"resulting_version":run.version+1}))]).await?;
    Ok(records[0].public())
}
pub async fn accepted(app: &App, owner: &str, session: Option<&str>) -> Result<Vec<Value>> {
    let mut runs = app.store.list(owner, "run").await?;
    runs.retain(|r| {
        r.data["review"]["decision"] == "accepted"
            && session.is_none_or(|s| r.data["session_id"] == s)
    });
    runs.sort_by(|a, b| a.id.cmp(&b.id));
    let rows=runs.iter().map(|r|json!({"schema_version":"1.0","run_id":r.id,"revision":r.version,"result":r.data["result"],"review":r.data["review"],"model":r.data["model"]})).collect::<Vec<_>>();
    let bytes = serde_json::to_string(&rows)?;
    let id = hash(&bytes);
    if app.store.get(owner, "export", &id).await.is_err() {
        app.store
            .put(Write::new(
                owner,
                "export",
                &id,
                json!({"sha256":id,"items":rows,"count":rows.len()}),
            ))
            .await?;
    }
    Ok(rows)
}
pub enum ExportKind {
    Sources,
    Accepted,
}
pub fn serialize(rows: &[Value], format: &str, kind: ExportKind) -> Result<(String, &'static str)> {
    match format {
        "json" => Ok((serde_json::to_string_pretty(rows)?, "application/json")),
        "jsonl" => Ok((
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
            "application/x-ndjson",
        )),
        "tsv" => {
            let mut lines = vec![match kind {
                ExportKind::Sources => "id\ttitle\ttext\tsource_url\tsha256\tlicence",
                ExportKind::Accepted => "run_id\trevision\tmodel\tanswer\tcitations_json\treview_decision\treview_reason\treview_actor\tevidence_only\tinterpretation_status\tverification_verdict",
            }.to_string()];
            for row in rows {
                let cells = match kind {
                    ExportKind::Sources => ["id", "title", "text", "url", "sha256", "licence"]
                        .iter()
                        .map(|k| row[*k].as_str().unwrap_or("").to_owned())
                        .collect::<Vec<_>>(),
                    ExportKind::Accepted => vec![
                        row["run_id"].as_str().unwrap_or("").into(),
                        row["revision"].to_string(),
                        row["model"].as_str().unwrap_or("").into(),
                        row["result"]["answer"].as_str().unwrap_or("").into(),
                        row["result"]["citations"].to_string(),
                        row["review"]["decision"].as_str().unwrap_or("").into(),
                        row["review"]["reason"].as_str().unwrap_or("").into(),
                        row["review"]["actor"].as_str().unwrap_or("").into(),
                        row["result"]["evidence_only"]
                            .as_bool()
                            .map(|value| value.to_string())
                            .unwrap_or_default(),
                        row["result"]["interpretation_status"]
                            .as_str()
                            .unwrap_or("")
                            .into(),
                        row["result"]["verification"]["verdict"]
                            .as_str()
                            .unwrap_or("")
                            .into(),
                    ],
                };
                lines.push(
                    cells
                        .iter()
                        .map(|value| tabular_cell(value))
                        .collect::<Vec<_>>()
                        .join("\t"),
                );
            }
            Ok((
                lines.join("\n") + "\n",
                "text/tab-separated-values; charset=utf-8",
            ))
        }
        _ => Err(Error::bad("Choose json, jsonl, or tsv")),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepted_tsv_preserves_review_and_citations() {
        let row = json!({"run_id":"run-1","revision":3,"model":"apertus-v1-5-8b","result":{"answer":"=a\tb\nc","citations":[{"source_id":"stapfer-2027","quote":"läsen"}]},"review":{"decision":"accepted","reason":"Checked against source","actor":"user-1"}});
        let (text, _) = serialize(&[row], "tsv", ExportKind::Accepted).unwrap();
        let lines = text.lines().collect::<Vec<_>>();
        let cells = lines[1].split('\t').collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        assert_eq!(cells.len(), 11);
        assert_eq!(&cells[..4], &["run-1", "3", "apertus-v1-5-8b", "'=a b c"]);
        assert_eq!(
            serde_json::from_str::<Value>(cells[4]).unwrap()[0]["source_id"],
            "stapfer-2027"
        );
        assert_eq!(
            &cells[5..8],
            &["accepted", "Checked against source", "user-1"]
        );
        assert_eq!(&cells[8..], &["", "", ""]);
    }
    #[test]
    fn empty_exports_keep_the_requested_columns() {
        assert!(
            serialize(&[], "tsv", ExportKind::Accepted)
                .unwrap()
                .0
                .starts_with("run_id\t")
        );
        assert!(
            serialize(&[], "tsv", ExportKind::Sources)
                .unwrap()
                .0
                .starts_with("id\ttitle\t")
        );
    }
    #[test]
    fn accepted_tsv_identifies_unverified_interpretations() {
        let row = json!({"run_id":"evidence-1","revision":4,"model":"apertus-v1-5-8b","result":{"answer":"Selected passages require human review.","citations":[{"source_id":"stapfer-2027","passage_id":"p048","quote":"läsen"}],"evidence_only":true,"interpretation_status":"needs_review","verification":{"verdict":"needs_revision"}},"review":{"decision":"accepted","reason":"Accept exact evidence only","actor":"reviewer"}});
        let (text, _) = serialize(&[row], "tsv", ExportKind::Accepted).unwrap();
        let mut lines = text.lines();
        let headers = lines.next().unwrap().split('\t').collect::<Vec<_>>();
        let cells = lines.next().unwrap().split('\t').collect::<Vec<_>>();
        assert_eq!(headers.len(), cells.len());
        for (name, expected) in [
            ("evidence_only", "true"),
            ("interpretation_status", "needs_review"),
            ("verification_verdict", "needs_revision"),
        ] {
            let index = headers.iter().position(|header| *header == name).unwrap();
            assert_eq!(cells[index], expected);
        }
    }
}
pub async fn dataset(app: &App, owner: &str, id: &str) -> Result<Vec<Value>> {
    let mut rows = sources::all(app, owner)
        .await?
        .into_iter()
        .filter(|r| r.data["dataset_id"] == id)
        .map(|r| r.public())
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    if rows.is_empty() {
        return Err(Error::missing());
    }
    Ok(rows)
}
