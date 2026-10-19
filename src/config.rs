#[derive(Clone)]
pub struct Config {
    pub bind: String,
    pub db_url: String,
    pub core_url: String,
    pub core_grpc_url: String,
    pub core_connect_seconds: u64,
    pub core_request_seconds: u64,
    pub covenant_url: Option<String>,
    pub token: String,
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
        Ok(Self {
            bind: std::env::var("BACK_BIND")?,
            db_url: std::env::var("OLDIF_DB_URL")?,
            core_url: std::env::var("OLDIF_CORE_URL")?,
            core_grpc_url: std::env::var("OLDIF_CORE_GRPC_URL")?,
            core_connect_seconds: std::env::var("CORE_CONNECT_TIMEOUT_SECONDS")
                .unwrap_or_else(|_| "5".into())
                .parse()?,
            core_request_seconds: std::env::var("CORE_REQUEST_TIMEOUT_SECONDS")
                .unwrap_or_else(|_| "20".into())
                .parse()?,
            covenant_url: std::env::var("OLDIF_COVENANT_URL")
                .ok()
                .filter(|s| !s.is_empty()),
            token,
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
