use crate::{
    application::auth::hash,
    error::{Error, Result},
    infrastructure::store::{Record, Write},
    state::App,
};
use serde_json::{Value, json};
use uuid::Uuid;

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

pub async fn import(app: &App, owner: &str, input: Value) -> Result<Value> {
    if input["rights_confirmed"] != true {
        return Err(Error::bad("Confirm that these records may be processed"));
    }
    let rows = input["records"]
        .as_array()
        .ok_or_else(|| Error::bad("records must be an array"))?;
    if rows.is_empty() || rows.len() > 200 {
        return Err(Error::bad("Import between 1 and 200 records"));
    }
    let id = Uuid::new_v4().to_string();
    let mut writes = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let text = row["text"]
            .as_str()
            .ok_or_else(|| Error::bad("Every record needs transcription text"))?;
        if text.len() > 100000 || text.trim().is_empty() {
            return Err(Error::bad(
                "Each transcription must contain 1 to 100000 bytes",
            ));
        }
        let url = row["url"].as_str().unwrap_or("");
        if !url.starts_with("https://") || reqwest::Url::parse(url).is_err() {
            return Err(Error::bad(
                "Every source needs a valid HTTPS provenance URL",
            ));
        }
        let source_id = format!("{id}-{index}");
        let data = json!({"id":source_id,"dataset_id":id,"title":row["title"].as_str().unwrap_or("Imported record"),"text":text,"normalized_text":text.replace("\r\n","\n"),"url":url,"page":row.get("page").filter(|p|p.is_u64()),"sha256":hash(text),"licence":input["licence"].as_str().unwrap_or("user-attested"),"record_locator":{"kind":"import-row","row":index+1},"transformation":{"operation":"line-ending-normalization","version":"1"},"provenance":"user-attested","flags":{"page_reference_missing":row["page"].is_null(),"handwriting_status":"not-assessed","legibility_status":"published-text"}});
        writes.push(Write::new(owner, "source", &source_id, data));
    }
    writes.push(Write::new(owner,"dataset",&id,json!({"name":input["name"].as_str().unwrap_or("Imported archival records"),"record_count":rows.len(),"licence":input["licence"],"kind":"user-import","provenance":"user-attested"})));
    app.store.batch(writes).await?;
    Ok(json!({"id":id,"record_count":rows.len(),"status":"imported"}))
}
