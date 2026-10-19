use crate::{
    application::{auth, document_tables, documents},
    error::{Error, Result},
    infrastructure::{document_parser::digest, document_structure, store::Write},
    state::App,
};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub ocr: bool,
    pub language: Option<String>,
    pub local_processing_confirmed: bool,
}
pub async fn extract(app: &App, owner: &str, id: &str, page: usize, input: Input) -> Result<Value> {
    if !input.local_processing_confirmed {
        return Err(Error::bad(
            "Confirm local page processing before extraction",
        ));
    }
    documents::page_detail(app, owner, id, page).await?;
    auth::limit(app, &format!("document-extraction:{owner}"), 10).await?;
    let _permit = app.document_slots.try_acquire().map_err(|_| {
        Error(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "Another document operation is running".into(),
        )
    })?;
    let (document, bytes) = documents::original(app, owner, id).await?;
    super::document_quarantine::require_use(&document,"local_processing")?;
    if document.data["media_type"] != "application/pdf" {
        return Err(Error::bad("Page geometry and OCR require a PDF document"));
    }
    let language = input.language.as_deref().unwrap_or("eng");
    let started = std::time::Instant::now();
    let extraction =
        document_structure::extract(&app.config.documents, &bytes, page, input.ocr, language)
            .await?;
    let extraction_id = uuid::Uuid::new_v4().to_string();
    let text = &extraction.geometry.text;
    let source_id = if input.ocr && !text.trim().is_empty() {
        Some(format!("{id}-ocr-{extraction_id}"))
    } else {
        None
    };
    let mut result = extraction.metadata;
    result["document_id"] = json!(id);
    result["page"] = json!(page);
    result["dimensions"] =
        json!({"width":extraction.geometry.width,"height":extraction.geometry.height});
    result["table_candidates"] = json!(document_tables::candidates(&extraction.geometry));
    result["words"] = json!(extraction.geometry.words);
    result["lines"] = json!(extraction.geometry.lines);
    result["text"] = json!(text);
    result["text_sha256"] = json!(digest(text.as_bytes()));
    result["original_sha256"] = document.data["original_sha256"].clone();
    result["ocr_source_id"] = json!(source_id);
    result["handwriting_status"] = json!("unknown");
    result["human_review_required"] = json!(true);
    result["processing_ms"] = json!(started.elapsed().as_millis());
    result["transformation"]=super::provenance::transformation(if input.ocr{"ocr_page"}else{"extract_text_geometry"},"Explicit local processing consent; transcription remains unverified",owner,&extraction_id,json!({"kind":"document","id":id,"version":document.version}),json!({"value_kind":"immutable_original_reference","document_id":id,"original_sha256":document.data["original_sha256"],"file_page_index":page-1}),json!({"value_kind":"immutable_record_reference","kind":"document_extraction","id":extraction_id,"version":1,"field":"text","text_sha256":result["text_sha256"]}));
    let mut writes = vec![Write::new(
        owner,
        "document_extraction",
        &extraction_id,
        result.clone(),
    )];
    if let Some(source_id) = source_id {
        writes.push(ocr_source(
            app,
            owner,
            &document.public(),
            page,
            &source_id,
            &extraction_id,
            &result,
        ));
    }
    let records = app.store.batch(writes).await?;
    Ok(json!({"structure":records[0].public()}))
}
fn ocr_source(
    app: &App,
    owner: &str,
    document: &Value,
    page: usize,
    source_id: &str,
    extraction_id: &str,
    result: &Value,
) -> Write {
    let id = document["id"].as_str().unwrap();
    let data = json!({"id":source_id,"document_id":id,"dataset_id":id,"title":format!("{} · OCR page {page}",document["title"].as_str().unwrap_or("Document")),"page":page,"file_page_index":page-1,"printed_page_label":null,"printed_page_label_missing_reason":"not_assessed","text":result["text"],"text_sha256":result["text_sha256"],"original_sha256":document["original_sha256"],"sha256":document["original_sha256"],"record_hash_scope":"original-file-bytes","url":format!("{}/api/documents/{id}/pages/{page}",app.config.origin),"licence":document["licence"],"derived_from":format!("{id}-p{page:04}"),"extraction_id":extraction_id,"transcription_method":"tesseract-ocr","transcription_review_status":"unverified","queryable_by_default":false,"transformation":result["transformation"],"flags":{"handwriting_status":"unknown","human_review_required":true,"ocr_performed":true}});
    Write::new(owner, "source", source_id, data)
}
pub async fn latest(app: &App, owner: &str, id: &str, page: usize) -> Result<Value> {
    documents::page_detail(app, owner, id, page).await?;
    let row = app
        .store
        .list(owner, "document_extraction")
        .await?
        .into_iter()
        .filter(|r| r.data["document_id"] == id && r.data["page"] == page)
        .max_by_key(|r| r.created_at)
        .ok_or_else(Error::missing)?;
    Ok(json!({"structure":row.public()}))
}
