use crate::{
    application::auth,
    error::{Error, Result},
    infrastructure::{
        document_parser::{self, Parsed},
        store::{Record, Write},
    },
    state::App,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub title: Option<String>,
    pub licence: String,
    pub rights_confirmed: bool,
}
pub async fn upload(
    app: &App,
    owner: &str,
    filename: &str,
    content_type: &str,
    input: Metadata,
    bytes: Vec<u8>,
) -> Result<Value> {
    validate_metadata(filename, &input)?;
    let media = document_parser::media(filename, content_type, &bytes)?;
    let _permit = app.document_slots.try_acquire().map_err(|_| {
        Error(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "Another document operation is running; retry shortly".into(),
        )
    })?;
    auth::limit(app, &format!("document:{owner}"), 5).await?;
    let started = std::time::Instant::now();
    let parsed = document_parser::parse(&app.config.documents, media, &bytes).await?;
    let id = Uuid::new_v4().to_string();
    let hash = document_parser::digest(&bytes);
    let data = document_data(&id, filename, media, &input, &bytes, &parsed, &hash);
    let document = app
        .store
        .put(Write::new(owner, "document", &id, data))
        .await?;
    let result = persist(app, &document, &bytes, &parsed, &hash).await;
    if result.is_err() {
        let mut failed = document.data.clone();
        failed["status"] = json!("failed");
        let _ = app.store.put(Write::update(&document, failed)).await;
    }
    let mut result = result?;
    result["processing_ms"] = json!(started.elapsed().as_millis());
    Ok(result)
}
fn validate_metadata(filename: &str, input: &Metadata) -> Result<()> {
    if !input.rights_confirmed
        || input.licence.trim().is_empty()
        || input.licence.len() > 200
        || filename.is_empty()
        || filename.len() > 180
        || filename.contains(['/', '\\', '\0', '\n', '\r'])
        || input.title.as_ref().is_some_and(|s| s.len() > 500)
    {
        return Err(Error::bad(
            "A bounded filename, licence, and processing rights confirmation are required",
        ));
    }
    Ok(())
}
fn document_data(
    id: &str,
    filename: &str,
    media: &str,
    input: &Metadata,
    bytes: &[u8],
    parsed: &Parsed,
    hash: &str,
) -> Value {
    let suggested = parsed
        .pages
        .iter()
        .flat_map(|p| p.lines())
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or(filename);
    let title = input
        .title
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(suggested)
        .chars()
        .take(200)
        .collect::<String>();
    let flags = parsed
        .pages
        .iter()
        .map(|text| page_flags(text))
        .collect::<Vec<_>>();
    let metadata: serde_json::Map<String, Value> = parsed
        .metadata
        .as_object()
        .into_iter()
        .flat_map(|m| m.iter())
        .filter(|(key, value)| key.starts_with("pdf_") && value.is_string())
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let title_supplied = input.title.as_ref().is_some_and(|value| !value.trim().is_empty());
    let metadata_method = if media == "application/pdf" { "embedded_pdf_metadata" } else { "none" };
    json!({"schema_version":"1.0","dataset_id":id,"title":title,"title_method":if title_supplied{"user_supplied"}else{"first_nonempty_extracted_line"},"filename":filename,"media_type":media,"original_sha256":hash,"byte_count":bytes.len(),"page_count":parsed.pages.len(),"status":"processing","licence":input.licence,"rights_confirmed":true,"labels":[],"annotations":[],"metadata":metadata,"metadata_provenance":{"method":metadata_method,"verification":"unverified_document_claim","original_sha256":hash},"extraction":parsed.metadata,"review":{"decision":"needs_review","reason":"New ingestion requires human review"},"enrichments":[],"flags":{"empty_pages":flags.iter().filter(|f|f["empty_text"]==true).count(),"suspect_pages":flags.iter().filter(|f|f["illegibility_suspected"]==true).count(),"pagination":"physical_page_order","handwriting_status":"unknown","ocr_performed":false,"privacy_review_required":true},"transformation":{"operation":"extract_text_without_modifying_original","version":"1","page_numbering":"one_based_physical_order"}})
}
pub fn page_flags(text: &str) -> Value {
    let empty = text.trim().is_empty();
    let suspect = !empty
        && text
            .chars()
            .filter(|c| *c == '\u{fffd}' || ('\u{e000}'..='\u{f8ff}').contains(c))
            .count()
            * 100
            > text.chars().count().max(1);
    json!({"empty_text":empty,"illegibility_suspected":suspect,"handwriting_status":"unknown","legibility_status":if empty{"empty_or_image_only"}else{"text_extracted"},"printed_page_label":"not_assessed","model_context_limit_exceeded":text.len()>80000,"human_review_required":true,"bias":"not_assessed","missingness":if empty{"no_extractable_text; image-only, blank, or other cause unknown"}else{"text_layer_only; visual omissions not assessed"}})
}
async fn persist(
    app: &App,
    document: &Record,
    bytes: &[u8],
    parsed: &Parsed,
    hash: &str,
) -> Result<Value> {
    let chunks = bytes.chunks(98_304).enumerate().map(|(index, chunk)| Write::new(&document.owner,"document_blob",&format!("{}-{index:04}",document.id),json!({"document_id":document.id,"index":index,"encoding":"base64","sha256":document_parser::digest(chunk),"bytes":STANDARD.encode(chunk)}))).collect::<Vec<_>>();
    for group in chunks.chunks(8) {
        app.store.batch(group.to_vec()).await?;
    }
    let pages = parsed
        .pages
        .iter()
        .enumerate()
        .map(|(i, text)| page(app, document, i + 1, text, hash))
        .collect::<Vec<_>>();
    for group in pages.chunks(8) {
        app.store
            .batch(
                group
                    .iter()
                    .map(|p| {
                        Write::new(
                            &document.owner,
                            "source",
                            p["id"].as_str().unwrap(),
                            p.clone(),
                        )
                    })
                    .collect(),
            )
            .await?;
    }
    let mut data = document.data.clone();
    data["status"] = json!("ready");
    data["blob_chunks"] = json!(chunks.len());
    let dataset = json!({"name":data["title"],"title":data["title"],"record_count":pages.len(),"licence":data["licence"],"kind":"uploaded-document","document_id":document.id,"page_citations":true,"provenance":"user-attested-original-upload"});
    let records = app
        .store
        .batch(vec![
            Write::update(document, data),
            Write::new(&document.owner, "dataset", &document.id, dataset),
        ])
        .await?;
    Ok(json!({"document":records[0].public(),"pages":pages}))
}
fn page(app: &App, document: &Record, number: usize, text: &str, hash: &str) -> Value {
    let id = format!("{}-p{number:04}", document.id);
    json!({"id":id,"document_id":document.id,"dataset_id":document.id,"title":format!("{} · page {number}",document.data["title"].as_str().unwrap_or("Document")),"text":text,"url":format!("{}/api/documents/{}/pages/{number}",app.config.origin,document.id),"page":number,"sha256":hash,"original_sha256":hash,"record_hash_scope":"original-file-bytes","text_sha256":document_parser::digest(text.as_bytes()),"licence":document.data["licence"],"record_locator":{"kind":"document-page","page":number,"document_id":document.id},"flags":page_flags(text),"provenance":"user-attested-original-upload"})
}
pub async fn get(app: &App, owner: &str, id: &str) -> Result<Record> {
    app.store.get(owner, "document", id).await
}
pub async fn pages(app: &App, owner: &str, id: &str) -> Result<Vec<Value>> {
    get(app, owner, id).await?;
    let mut pages = app
        .store
        .list(owner, "source")
        .await?
        .into_iter()
        .filter(|r| r.data["document_id"] == id && r.data["derived_from"].is_null())
        .map(|r| r.public())
        .collect::<Vec<_>>();
    pages.sort_by_key(|p| p["page"].as_u64());
    Ok(pages)
}
pub async fn detail(app: &App, owner: &str, id: &str) -> Result<Value> {
    let document = get(app, owner, id).await?;
    let revisions = app
        .store
        .list(owner, "review")
        .await?
        .into_iter()
        .filter(|r| r.data["document_id"] == id)
        .map(|r| r.public())
        .collect::<Vec<_>>();
    let derived_sources=app.store.list(owner,"source").await?.into_iter().filter(|r|r.data["document_id"]==id&&r.data["derived_from"].is_string()).map(|r|r.public()).collect::<Vec<_>>();
    let extractions=app.store.list(owner,"document_extraction").await?.into_iter().filter(|r|r.data["document_id"]==id).map(|r|r.public()).collect::<Vec<_>>();
    Ok(json!({"document":document.public(),"pages":pages(app,owner,id).await?,"revisions":revisions,"derived_sources":derived_sources,"extractions":extractions}))
}
pub async fn original(app: &App, owner: &str, id: &str) -> Result<(Record, Vec<u8>)> {
    let document = get(app, owner, id).await?;
    let count = document.data["blob_chunks"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= 171)
        .ok_or_else(Error::unavailable)?;
    let mut bytes = Vec::new();
    for index in 0..count {
        let chunk = app
            .store
            .get(owner, "document_blob", &format!("{id}-{index:04}"))
            .await?;
        let data = STANDARD
            .decode(
                chunk.data["bytes"]
                    .as_str()
                    .ok_or_else(Error::unavailable)?,
            )
            .map_err(|_| Error::unavailable())?;
        if document_parser::digest(&data) != chunk.data["sha256"] {
            return Err(Error::unavailable());
        }
        bytes.extend(data);
        if bytes.len() > app.config.documents.max_bytes {
            return Err(Error::unavailable());
        }
    }
    if document_parser::digest(&bytes) != document.data["original_sha256"] {
        return Err(Error::unavailable());
    }
    Ok((document, bytes))
}
pub async fn page_detail(app: &App, owner: &str, id: &str, number: usize) -> Result<Value> {
    let doc = get(app, owner, id).await?;
    if number == 0 || number > doc.data["page_count"].as_u64().unwrap_or(0) as usize {
        return Err(Error::missing());
    }
    let page = app
        .store
        .get(owner, "source", &format!("{id}-p{number:04}"))
        .await?;
    let review = doc.data["annotations"]
        .as_array()
        .and_then(|a| a.iter().find(|a| a["page"] == number))
        .cloned();
    Ok(json!({"page":page.public(),"review":review}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flags_do_not_claim_ocr_or_handwriting_recognition() {
        assert_eq!(page_flags("")["handwriting_status"], "unknown");
        assert_eq!(page_flags("")["legibility_status"], "empty_or_image_only");
        assert_eq!(page_flags("ordinary text")["illegibility_suspected"], false);
        assert_eq!(
            page_flags("\u{fffd}\u{fffd}x")["illegibility_suspected"],
            true
        );
    }
}
