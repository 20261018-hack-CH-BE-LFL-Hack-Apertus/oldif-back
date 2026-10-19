use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
#[derive(Debug)]
pub struct Error(pub StatusCode, pub String);
pub type Result<T> = std::result::Result<T, Error>;
impl Error {
    pub fn bad(message: &str) -> Self {
        Self(StatusCode::BAD_REQUEST, message.into())
    }
    pub fn unauthorized() -> Self {
        Self(StatusCode::UNAUTHORIZED, "Sign in to continue".into())
    }
    pub fn missing() -> Self {
        Self(StatusCode::NOT_FOUND, "Resource not found".into())
    }
    pub fn unavailable() -> Self {
        Self(
            StatusCode::SERVICE_UNAVAILABLE,
            "A required service is unavailable; please retry".into(),
        )
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":self.1}))).into_response()
    }
}
impl From<reqwest::Error> for Error {
    fn from(_: reqwest::Error) -> Self {
        Self::unavailable()
    }
}
impl From<serde_json::Error> for Error {
    fn from(_: serde_json::Error) -> Self {
        Self::bad("Invalid JSON response")
    }
}
impl From<tonic::Status> for Error {
    fn from(_: tonic::Status) -> Self {
        Self::unavailable()
    }
}
impl From<tonic::transport::Error> for Error {
    fn from(_: tonic::transport::Error) -> Self {
        Self::unavailable()
    }
}
