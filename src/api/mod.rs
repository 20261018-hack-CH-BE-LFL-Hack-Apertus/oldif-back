pub mod archive;
pub mod auth;
pub mod realtime;
use crate::{error::Result, state::App};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderValue, Method},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
};
use serde_json::{Value, json};
pub fn router(app: App) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/health", get(health))
        .route("/api/auth/demo", post(auth::demo))
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/session", get(auth::session))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/models", get(archive::models))
        .route("/api/datasets", get(archive::datasets))
        .route("/api/datasets/{id}/sources", get(archive::dataset_sources))
        .route("/api/datasets/{id}/export", get(archive::dataset_export))
        .route("/api/sources/{id}", get(archive::source))
        .route("/api/search", get(archive::search))
        .route("/api/import", post(archive::import))
        .route(
            "/api/sessions",
            get(archive::sessions).post(archive::create_session),
        )
        .route("/api/sessions/{id}", get(archive::session))
        .route("/api/runs", post(archive::run))
        .route("/api/runs/{id}/review", post(archive::review))
        .route("/api/runs/{id}/steer", post(archive::steer))
        .route("/api/runs/{id}/cancel", post(archive::cancel))
        .route("/api/export", get(archive::export))
        .route("/api/runs/{id}/agent-review", post(archive::agent_review))
        .route("/api/mcp", post(archive::mcp))
        .route("/api/voice/session", post(realtime::voice))
        .route("/api/realtime", get(realtime::socket))
        .route_layer(middleware::from_fn_with_state(app.clone(), secure))
        .layer(DefaultBodyLimit::max(4_000_000))
        .with_state(app)
}
async fn health() -> Json<Value> {
    Json(json!({"status":"ok","service":"oldif-back"}))
}
async fn secure(State(app): State<App>, request: Request, next: Next) -> Result<Response> {
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        crate::application::auth::origin(&app, request.headers())?;
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}
