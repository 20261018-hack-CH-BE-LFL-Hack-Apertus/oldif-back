use crate::{
    application::{auth, document_export, document_review, documents},
    error::{Error, Result},
    infrastructure::document_parser,
    state::App,
};
use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

pub async fn list(State(app): State<App>, headers: HeaderMap) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    let rows = app.store.list(&user.id, "document").await?;
    Ok(Json(
        json!({"documents":rows.iter().map(|r|r.public()).collect::<Vec<_>>()}),
    ))
}
pub async fn upload(
    State(app): State<App>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    let filename = headers
        .get("x-document-filename")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| Error::bad("X-Document-Filename is required"))?;
    let metadata = headers
        .get("x-document-metadata")
        .and_then(|h| h.to_str().ok())
        .filter(|s| s.len() <= 4000)
        .ok_or_else(|| Error::bad("X-Document-Metadata JSON is required"))?;
    let metadata: documents::Metadata =
        serde_json::from_str(metadata).map_err(|_| Error::bad("Invalid document metadata JSON"))?;
    let media = headers
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    Ok(Json(
        documents::upload(&app, &user.id, filename, media, metadata, body.to_vec()).await?,
    ))
}
pub async fn detail(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    Ok(Json(documents::detail(&app, &user.id, &id).await?))
}
pub async fn page(
    State(app): State<App>,
    headers: HeaderMap,
    Path((id, page)): Path<(String, usize)>,
) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    Ok(Json(
        documents::page_detail(&app, &user.id, &id, page).await?,
    ))
}
pub async fn original(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response> {
    let user = auth::authenticate(&app, &headers).await?;
    let (document, bytes) = documents::original(&app, &user.id, &id).await?;
    let extension = match document.data["media_type"].as_str() {
        Some("application/pdf") => "pdf",
        Some("application/json") => "json",
        _ => "txt",
    };
    download(
        bytes,
        "application/octet-stream",
        &format!("oldif-original-{id}.{extension}"),
    )
}
pub async fn preview(
    State(app): State<App>,
    headers: HeaderMap,
    Path((id, page)): Path<(String, usize)>,
) -> Result<Response> {
    let user = auth::authenticate(&app, &headers).await?;
    documents::page_detail(&app, &user.id, &id, page).await?;
    let _permit = app.document_slots.try_acquire().map_err(|_| {
        Error(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "Another document operation is running".into(),
        )
    })?;
    let (document, bytes) = documents::original(&app, &user.id, &id).await?;
    if document.data["media_type"] != "application/pdf" {
        return Err(Error::missing());
    }
    let png = document_parser::preview(&app.config.documents, &bytes, page).await?;
    Ok(([("content-type", "image/png")], png).into_response())
}
pub async fn review(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<document_review::Review>,
) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    Ok(Json(
        document_review::review(&app, &user.id, &id, input).await?,
    ))
}
pub async fn enrich(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<document_review::Enrichment>,
) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    Ok(Json(
        document_review::enrich(&app, &user, &id, input).await?,
    ))
}
#[derive(Deserialize)]
pub struct ExportQuery {
    pub format: Option<String>,
}
pub async fn export(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(input): Query<ExportQuery>,
) -> Result<Response> {
    let user = auth::authenticate(&app, &headers).await?;
    let format = input.format.as_deref().unwrap_or("json");
    let (body, media) = document_export::export(&app, &user.id, &id, format).await?;
    download(
        body.into_bytes(),
        media,
        &format!(
            "oldif-document-{id}.{}",
            if format == "manifest" {
                "manifest.json"
            } else {
                format
            }
        ),
    )
}
fn download(bytes: Vec<u8>, media: &'static str, filename: &str) -> Result<Response> {
    let mut response = bytes.into_response();
    response
        .headers_mut()
        .insert("content-type", HeaderValue::from_static(media));
    response.headers_mut().insert(
        "content-disposition",
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\" "))
            .map_err(|_| Error::unavailable())?,
    );
    Ok(response)
}
