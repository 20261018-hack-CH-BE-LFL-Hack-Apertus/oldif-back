use crate::{
    application::{auth, document_extraction},
    error::Result,
    state::App,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde_json::Value;
pub async fn extract(
    State(app): State<App>,
    headers: HeaderMap,
    Path((id, page)): Path<(String, usize)>,
    Json(input): Json<document_extraction::Input>,
) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    Ok(Json(
        document_extraction::extract(&app, &user.id, &id, page, input).await?,
    ))
}
pub async fn structure(
    State(app): State<App>,
    headers: HeaderMap,
    Path((id, page)): Path<(String, usize)>,
) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    Ok(Json(
        document_extraction::latest(&app, &user.id, &id, page).await?,
    ))
}
