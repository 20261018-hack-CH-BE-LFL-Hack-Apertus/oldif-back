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
    pub fn timeout() -> Self { Self(StatusCode::GATEWAY_TIMEOUT,"The operation deadline expired".into()) }
    pub fn cancelled() -> Self { Self(StatusCode::from_u16(499).unwrap(),"The operation was cancelled".into()) }
    pub fn backpressure() -> Self { Self(StatusCode::TOO_MANY_REQUESTS,"The service is at its bounded capacity".into()) }
    pub fn code(&self)->&'static str {
        match self.0.as_u16() {
            408|504=>"TIMEOUT",499=>"CANCELLED",429=>"BACKPRESSURE",
            409 if self.1.starts_with("IDEMPOTENCY_CONFLICT")=>"IDEMPOTENCY_CONFLICT",
            409=>"REVISION_CONFLICT",400|422=>"INVALID_REQUEST",401=>"UNAUTHENTICATED",
            403=>"FORBIDDEN",404=>"NOT_FOUND",413=>"PAYLOAD_TOO_LARGE",503=>"UNAVAILABLE",_=>"INTERNAL_ERROR",
        }
    }
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
        (self.0, Json(json!({"error":self.1,"code":self.code()}))).into_response()
    }
}
impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        if error.is_timeout(){Self::timeout()}else{Self::unavailable()}
    }
}
impl From<serde_json::Error> for Error {
    fn from(_: serde_json::Error) -> Self {
        Self::bad("Invalid JSON response")
    }
}
impl From<tonic::Status> for Error {
    fn from(error: tonic::Status) -> Self {
        let mut source=std::error::Error::source(&error);
        while let Some(cause)=source {if cause.is::<tonic::TimeoutExpired>(){return Self::timeout();}source=cause.source();}
        match error.code(){tonic::Code::DeadlineExceeded=>Self::timeout(),tonic::Code::Cancelled=>Self::cancelled(),tonic::Code::ResourceExhausted=>Self::backpressure(),tonic::Code::AlreadyExists|tonic::Code::Aborted=>Self(StatusCode::CONFLICT,"Reload the current revision".into()),_=>Self::unavailable()}
    }
}
impl From<tonic::transport::Error> for Error {
    fn from(_: tonic::transport::Error) -> Self {
        Self::unavailable()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn terminal_and_conflict_errors_have_stable_machine_codes() {
        for (error,code) in [(Error::timeout(),"TIMEOUT"),(Error::cancelled(),"CANCELLED"),(Error::backpressure(),"BACKPRESSURE"),(Error(StatusCode::CONFLICT,"stale".into()),"REVISION_CONFLICT"),(Error(StatusCode::CONFLICT,"IDEMPOTENCY_CONFLICT: different request".into()),"IDEMPOTENCY_CONFLICT")] {
            let response=error.into_response();let bytes=axum::body::to_bytes(response.into_body(),1024).await.unwrap();
            let body:serde_json::Value=serde_json::from_slice(&bytes).unwrap();assert_eq!(body["code"],code);assert!(body["error"].is_string());
        }
        assert_eq!(Error::from(tonic::Status::deadline_exceeded("private diagnostic")).code(),"TIMEOUT");
        assert_eq!(Error::from(tonic::Status::cancelled("private diagnostic")).code(),"CANCELLED");
        assert_eq!(Error::from(tonic::Status::resource_exhausted("private diagnostic")).code(),"BACKPRESSURE");
    }
}
