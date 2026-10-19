use crate::{
    application::auth,
    error::{Error, Result},
    state::App,
};
use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde_json::Value;

pub async fn capabilities(State(app): State<App>, headers: HeaderMap) -> Result<Json<Value>> {
    auth::authenticate(&app, &headers).await?;
    let response = app
        .client
        .get(format!("{}/v1/diarization", app.config.core_url))
        .bearer_auth(&app.config.core_token)
        .send()
        .await?;
    checked(response).await
}
pub async fn analyze(
    State(app): State<App>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    validate(&headers, body.len())?;
    auth::limit(&app, &format!("diarization:{}", user.id), 3).await?;
    let response = app
        .client
        .post(format!("{}/v1/diarization", app.config.core_url))
        .bearer_auth(&app.config.core_token)
        .header("Content-Type", "audio/wav")
        .header("X-Local-Processing-Confirmed", "true")
        .body(body)
        .send()
        .await?;
    checked(response).await
}
fn validate(headers: &HeaderMap, length: usize) -> Result<()> {
    if headers
        .get("x-local-processing-confirmed")
        .and_then(|v| v.to_str().ok())
        != Some("true")
    {
        return Err(Error::bad(
            "Confirm local audio processing before diarization",
        ));
    }
    if headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next())
        != Some("audio/wav")
    {
        return Err(Error::bad("Upload a PCM16 mono 16kHz WAV file"));
    }
    if length == 0 || length > 2_900_000 {
        return Err(Error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Audio must contain at most 90 seconds within the 2.9MB limit".into(),
        ));
    }
    Ok(())
}
async fn checked(response: reqwest::Response) -> Result<Json<Value>> {
    let status = response.status();
    if status.is_success() {
        return Ok(Json(response.json().await?));
    }
    let message = match status {
        StatusCode::BAD_REQUEST => "Audio must be PCM16 mono 16kHz WAV, no longer than 90 seconds",
        StatusCode::PAYLOAD_TOO_LARGE => "Audio exceeds the configured byte limit",
        _ => "Diarization is disabled, busy, or could not complete within its resource budget",
    };
    let public_status = if matches!(
        status,
        StatusCode::BAD_REQUEST | StatusCode::PAYLOAD_TOO_LARGE
    ) {
        status
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    Err(Error(public_status, message.into()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audio_requires_consent_type_and_bound_before_forwarding() {
        let mut h = HeaderMap::new();
        assert!(validate(&h, 100).is_err());
        h.insert("x-local-processing-confirmed", "true".parse().unwrap());
        assert!(validate(&h, 100).is_err());
        h.insert("content-type", "audio/wav".parse().unwrap());
        assert!(validate(&h, 100).is_ok());
        assert_eq!(
            validate(&h, 2_900_001).unwrap_err().0,
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }
}
