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
        })
    }
}
