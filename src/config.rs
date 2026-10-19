#[derive(Clone)]
pub struct Config {
    pub bind: String,
    pub db_url: String,
    pub db_grpc_url: Option<String>,
    pub db_connect_seconds: u64,
    pub db_request_seconds: u64,
    pub core_url: String,
    pub core_grpc_url: String,
    pub core_connect_seconds: u64,
    pub core_request_seconds: u64,
    pub covenant_url: Option<String>,
    pub token: String,
    pub core_token: String,
    pub covenant_token: Option<String>,
    pub origin: String,
    pub secure_cookie: bool,
    pub session_seconds: u64,
    pub max_active: usize,
    pub documents: DocumentConfig,
}
#[derive(Clone)]
pub struct DocumentConfig {
    pub max_bytes: usize,
    pub max_pages: usize,
    pub max_text_bytes: usize,
    pub timeout_seconds: u64,
    pub memory_mb: u64,
    pub poppler_dir: std::path::PathBuf,
    pub tesseract: std::path::PathBuf,
}
impl DocumentConfig {
    fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            max_bytes: number("DOCUMENT_MAX_BYTES", 16_777_216)?.clamp(1024, 16_777_216) as usize,
            max_pages: number("DOCUMENT_MAX_PAGES", 300)?.clamp(1, 300) as usize,
            max_text_bytes: number("DOCUMENT_MAX_TEXT_BYTES", 8_388_608)?.clamp(1024, 8_388_608)
                as usize,
            timeout_seconds: number("DOCUMENT_TIMEOUT_SECONDS", 30)?.clamp(1, 60),
            memory_mb: number("DOCUMENT_MEMORY_MB", 512)?.clamp(128, 1024),
            poppler_dir: std::env::var("POPPLER_BIN_DIR")
                .unwrap_or_else(|_| "/usr/bin".into())
                .into(),
            tesseract: std::env::var("TESSERACT_EXECUTABLE")
                .unwrap_or_else(|_| "/usr/bin/tesseract".into())
                .into(),
        })
    }
}
fn number(name: &str, default: u64) -> Result<u64, Box<dyn std::error::Error>> {
    Ok(std::env::var(name)
        .unwrap_or_else(|_| default.to_string())
        .parse()?)
}
impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let token = std::env::var("INTERNAL_SERVICE_TOKEN")?;
        if token.len() < 32 {
            return Err("INTERNAL_SERVICE_TOKEN must be at least 32 characters".into());
        }
        let origin = std::env::var("APP_ORIGIN")?;
        let allow_local = std::env::var("OLDIF_ALLOW_SHARED_LOCAL_TOKEN").as_deref() == Ok("true")
            && local_origin(&origin);
        let core_token = scoped_token(
            "OLDIF_CORE_TOKEN",
            std::env::var("OLDIF_CORE_TOKEN").ok(),
            &token,
            allow_local,
        )?;
        let covenant_url = std::env::var("OLDIF_COVENANT_URL")
            .ok()
            .filter(|s| !s.is_empty());
        let covenant_token = if covenant_url.is_some() {
            Some(scoped_token(
                "OLDIF_COVENANT_TOKEN",
                std::env::var("OLDIF_COVENANT_TOKEN").ok(),
                &token,
                allow_local,
            )?)
        } else {
            None
        };
        if !allow_local && covenant_token.as_deref() == Some(&core_token) {
            return Err("Core and covenant credentials must differ".into());
        }
        let db_grpc_url = std::env::var("OLDIF_DB_GRPC_URL")
            .ok()
            .filter(|url| !url.trim().is_empty());
        if let Some(url) = &db_grpc_url {
            tonic::transport::Endpoint::from_shared(url.clone())?;
        }
        Ok(Self {
            bind: std::env::var("BACK_BIND")?,
            db_url: std::env::var("OLDIF_DB_URL")?,
            db_grpc_url,
            db_connect_seconds: number("DB_CONNECT_TIMEOUT_SECONDS", 5)?.clamp(1, 120),
            db_request_seconds: number("DB_REQUEST_TIMEOUT_SECONDS", 120)?.clamp(1, 120),
            core_url: std::env::var("OLDIF_CORE_URL")?,
            core_grpc_url: std::env::var("OLDIF_CORE_GRPC_URL")?,
            core_connect_seconds: std::env::var("CORE_CONNECT_TIMEOUT_SECONDS")
                .unwrap_or_else(|_| "5".into())
                .parse()?,
            core_request_seconds: std::env::var("CORE_REQUEST_TIMEOUT_SECONDS")
                .unwrap_or_else(|_| "20".into())
                .parse()?,
            covenant_url,
            token,
            core_token,
            covenant_token,
            secure_cookie: origin.starts_with("https://"),
            origin,
            session_seconds: std::env::var("SESSION_SECONDS")
                .unwrap_or_else(|_| "86400".into())
                .parse()?,
            max_active: std::env::var("MAX_ACTIVE_RUNS")
                .unwrap_or_else(|_| "4".into())
                .parse()?,
            documents: DocumentConfig::from_env()?,
        })
    }
}

fn local_origin(origin: &str) -> bool {
    reqwest::Url::parse(origin).is_ok_and(|url| {
        url.scheme() == "http"
            && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
    })
}
fn scoped_token(
    name: &str,
    value: Option<String>,
    database: &str,
    allow_local: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let value = value
        .or_else(|| allow_local.then(|| database.to_owned()))
        .ok_or_else(|| {
            format!("Set {name}; shared credentials require explicit local-only configuration")
        })?;
    if value.len() < 32 || value.starts_with("replace-") || (!allow_local && value == database) {
        return Err(format!(
            "{name} must contain at least 32 characters and differ from the database credential"
        )
        .into());
    }
    Ok(value)
}
#[cfg(test)]
mod credential_tests {
    use super::*;
    #[test]
    fn production_credentials_are_required_and_distinct() {
        let database = "d".repeat(32);
        assert!(scoped_token("test", None, &database, false).is_err());
        assert!(scoped_token("test", Some(database.clone()), &database, false).is_err());
        assert!(scoped_token("test", Some("c".repeat(32)), &database, false).is_ok());
        assert_eq!(
            scoped_token("test", None, &database, true).unwrap(),
            database
        );
    }
    #[test]
    fn shared_test_credentials_are_restricted_to_explicit_loopback_http() {
        for origin in [
            "http://localhost:3000",
            "http://127.0.0.1:18090",
            "http://[::1]:3000",
        ] {
            assert!(local_origin(origin));
        }
        for origin in [
            "https://oldif.example",
            "http://localhost.example",
            "http://example.org",
            "garbage",
        ] {
            assert!(!local_origin(origin));
        }
    }
}
