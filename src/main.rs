mod api;
mod application;
mod config;
mod domain;
mod error;
mod infrastructure;
mod state;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .json()
        .with_env_filter("info")
        .init();
    let config = config::Config::from_env()?;
    let bind = config.bind.clone();
    let app = state::App::new(config)?;
    application::sources::seed(&app)
        .await
        .map_err(|e| format!("Source initialization failed: {}", e.1))?;
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(service = "oldif-back", "backend ready");
    axum::serve(
        listener,
        api::router(app).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}
