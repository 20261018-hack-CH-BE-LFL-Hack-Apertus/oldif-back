use crate::{
    error::{Error, Result},
    infrastructure::store::Write,
    state::App,
};
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
    pub demo: bool,
}
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
pub fn hash(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn origin(app: &App, headers: &HeaderMap) -> Result<()> {
    if headers.get("origin").and_then(|h| h.to_str().ok()) != Some(app.config.origin.as_str()) {
        return Err(Error(
            StatusCode::FORBIDDEN,
            "Request origin not allowed".into(),
        ));
    }
    Ok(())
}
pub async fn limit(app: &App, key: &str, max: u32) -> Result<()> {
    let mut limits = app.limits.lock().await;
    let current = now();
    limits.retain(|_, v| current.saturating_sub(v.0) < 60);
    if limits.len() >= 10000 {
        return Err(Error(
            StatusCode::TOO_MANY_REQUESTS,
            "Please retry in a minute".into(),
        ));
    }
    let value = limits.entry(key.into()).or_insert((current, 0));
    value.1 += 1;
    if value.1 > max {
        return Err(Error(
            StatusCode::TOO_MANY_REQUESTS,
            "Please retry in a minute".into(),
        ));
    }
    Ok(())
}
pub async fn authenticate(app: &App, headers: &HeaderMap) -> Result<User> {
    let cookie = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let token = cookie
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find(|(name, _)| *name == "oldif_session")
        .map(|(_, value)| value)
        .ok_or_else(Error::unauthorized)?;
    if token.len() != 64 {
        return Err(Error::unauthorized());
    }
    let record = app
        .store
        .get("auth", "auth_session", &hash(token))
        .await
        .map_err(|_| Error::unauthorized())?;
    if record.data["expires_at"].as_u64().unwrap_or(0) <= now() || record.data["revoked"] == true {
        return Err(Error::unauthorized());
    }
    Ok(serde_json::from_value(record.data["user"].clone())?)
}
pub async fn create_session(app: &App, user: User) -> Result<(User, String)> {
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    app.store
        .put(Write::new(
            "auth",
            "auth_session",
            &hash(&token),
            json!({"user":user,"expires_at":now()+app.config.session_seconds,"revoked":false}),
        ))
        .await?;
    let secure = if app.config.secure_cookie {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "oldif_session={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
        app.config.session_seconds, secure
    );
    Ok((user, cookie))
}
pub async fn demo(app: &App) -> Result<(User, String)> {
    create_session(
        app,
        User {
            id: Uuid::new_v4().to_string(),
            name: "Demo researcher".into(),
            email: None,
            demo: true,
        },
    )
    .await
}
pub async fn register(app: &App, input: Value) -> Result<(User, String)> {
    let email = input["email"].as_str().unwrap_or("").trim().to_lowercase();
    let password = input["password"].as_str().unwrap_or("");
    if !email.contains('@') || email.len() > 254 || password.len() < 12 || password.len() > 256 {
        return Err(Error::bad(
            "Use a valid email and a password between 12 and 256 characters",
        ));
    }
    let user = User {
        id: Uuid::new_v4().to_string(),
        name: input["name"]
            .as_str()
            .unwrap_or("Researcher")
            .chars()
            .take(80)
            .collect(),
        email: Some(email.clone()),
        demo: false,
    };
    let password = password.to_owned();
    let encoded = tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
            .map(|h| h.to_string())
    })
    .await
    .map_err(|_| Error::unavailable())?
    .map_err(|_| Error::unavailable())?;
    app.store
        .put(Write::new(
            "auth",
            "account",
            &hash(&email),
            json!({"user":user,"password_hash":encoded}),
        ))
        .await?;
    create_session(app, user).await
}
pub async fn login(app: &App, input: Value) -> Result<(User, String)> {
    let email = input["email"].as_str().unwrap_or("").trim().to_lowercase();
    let password = input["password"].as_str().unwrap_or("").to_owned();
    if password.len() > 256 || email.len() > 254 {
        return Err(Error::unauthorized());
    }
    let record = app
        .store
        .get("auth", "account", &hash(&email))
        .await
        .map_err(|_| Error::unauthorized())?;
    let encoded = record.data["password_hash"]
        .as_str()
        .ok_or_else(Error::unauthorized)?
        .to_owned();
    let verified = tokio::task::spawn_blocking(move || {
        PasswordHash::new(&encoded).is_ok_and(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        })
    })
    .await
    .map_err(|_| Error::unavailable())?;
    if !verified {
        return Err(Error::unauthorized());
    }
    create_session(app, serde_json::from_value(record.data["user"].clone())?).await
}
pub async fn logout(app: &App, headers: &HeaderMap) -> Result<()> {
    let cookie = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if let Some((_, token)) = cookie
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find(|(n, _)| *n == "oldif_session")
    {
        if let Ok(record) = app.store.get("auth", "auth_session", &hash(token)).await {
            let mut data = record.data.clone();
            data["revoked"] = json!(true);
            app.store.put(Write::update(&record, data)).await?;
        }
    }
    Ok(())
}
