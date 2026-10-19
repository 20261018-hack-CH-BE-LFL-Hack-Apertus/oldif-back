use crate::{
    application::auth::hash,
    error::{Error, Result},
    infrastructure::store::{Record, Write},
    state::App,
};
use serde_json::{Value, json};

pub async fn seed(app: &App) -> Result<()> {
    let rows: Vec<Value> = include_str!("../../data/stapfer-sources.jsonl")
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    if app
        .store
        .get("public", "dataset", "stapfer-1799")
        .await
        .is_ok()
    {
        return super::stapfer_seed::seed(app).await;
    }
    let dataset = json!({"name":"Stapfer-Enquête 1799","title":"Stapfer-Enquête 1799","description":"105 published school-survey transcriptions supplied by the University of Bern challenge","record_count":rows.len(),"licence":"terms_open","source_url":"https://www.stapferenquete.ch/","kind":"published-transcription","page_citations":false,"provenance":"supplied-challenge-workbook"});
    let mut writes = vec![Write::new("public", "dataset", "stapfer-1799", dataset)];
    for row in rows {
        let id = row["id"]
            .as_str()
            .ok_or_else(|| Error::bad("Invalid bundled source"))?
            .to_owned();
        writes.push(Write::new("public", "source", &id, row));
    }
    app.store.batch(writes).await?;
    super::stapfer_seed::seed(app).await
}
pub async fn datasets(app: &App, owner: &str) -> Result<Vec<Value>> {
    let mut rows = app.store.list("public", "dataset").await?;
    rows.extend(app.store.list(owner, "dataset").await?);
    Ok(rows.iter().map(Record::public).collect())
}
pub async fn all(app: &App, owner: &str) -> Result<Vec<Record>> {
    let mut rows = app.store.list("public", "source").await?;
    rows.extend(app.store.list(owner, "source").await?);
    Ok(rows)
}
pub async fn get(app: &App, owner: &str, id: &str) -> Result<Record> {
    match app.store.get(owner, "source", id).await {
        Ok(row) => Ok(row),
        Err(e) if e.0 == axum::http::StatusCode::NOT_FOUND => {
            app.store.get("public", "source", id).await
        }
        Err(e) => Err(e),
    }
}
pub async fn search(
    app: &App,
    owner: &str,
    question: &str,
    datasets: &[String],
) -> Result<Vec<Value>> {
    if question.chars().count() > 8000 {
        return Err(Error::bad("Search query exceeds 8000 characters"));
    }
    let records = all(app, owner)
        .await?
        .into_iter()
        .filter(|r| {
            r.data["queryable_by_default"]!=false && !r.data["text"].as_str().unwrap_or("").trim().is_empty() && (datasets.is_empty() || datasets.iter().any(|d| r.data["dataset_id"] == d.as_str()))
        })
        .collect::<Vec<_>>();
    let documents = records
        .iter()
        .map(|r| crate::domain::retrieval::Document {
            id: r.id.clone(),
            title: r.data["title"].as_str().unwrap_or("").into(),
            location: r.data["metadata"]["Schulort"].as_str().unwrap_or("").into(),
            text: r.data["text"].as_str().unwrap_or("").into(),
        })
        .collect::<Vec<_>>();
    let selection = crate::domain::retrieval::select(question, &documents, 6);
    Ok(selection
        .indices
        .into_iter()
        .map(|(index, score)| {
            let mut row = records[index].public();
            row["retrieval_score"] = json!(score);
            row["retrieval_method"] = json!(selection.method);
            row["text_sha256"] = json!(hash(row["text"].as_str().unwrap_or("")));
            row["record_hash_scope"] = row.get("record_hash_scope").cloned().unwrap_or_else(|| json!(if row["metadata"].is_object() {
                "canonical-source-row-json"
            } else {
                "utf8-original-text"
            }));
            row
        })
        .collect())
}
pub async fn select(app:&App,owner:&str,question:&str,datasets:&[String],ids:&[String])->Result<Vec<Value>> {
    if ids.is_empty() { return search(app,owner,question,datasets).await; }
    let mut selected=vec![];
    let mut seen=std::collections::HashSet::new();
    for id in ids {
        if !seen.insert(id) { return Err(Error::bad("Source selection contains duplicates")); }
        let row=get(app,owner,id).await?.public();
        if row["text"].as_str().unwrap_or("").trim().is_empty() || (!datasets.is_empty()&&!datasets.iter().any(|d|row["dataset_id"]==d.as_str())) { return Err(Error::bad("Selected source is empty or outside the selected collection")); }
        let mut row=row; row["retrieval_method"]=json!("explicit-source-selection");
        row["text_sha256"]=json!(hash(row["text"].as_str().unwrap_or("")));
        selected.push(row);
    }
    Ok(selected)
}
