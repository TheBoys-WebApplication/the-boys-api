use axum::Router;
use sqlx::PgPool;
use std::sync::Arc;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

mod amadeus;
mod config;
mod db;
mod error;
mod jwt;
mod middleware;
mod routes;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub jwt_secret: String,
    pub amadeus_client: Arc<amadeus::AmadeusClient>,
    pub discover_cache: amadeus::DiscoverCache,
    pub invite_code: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = config::Config::from_env()?;
    let db = db::connect(&config.database_url).await?;

    sqlx::migrate!("./migrations").run(&db).await?;
    tracing::info!("migrations applied");

    let state = AppState {
        db,
        jwt_secret: config.jwt_secret,
        amadeus_client: Arc::new(amadeus::AmadeusClient::new(
            config.amadeus_key,
            config.amadeus_secret,
        )),
        discover_cache: amadeus::new_discover_cache(),
        invite_code: config.invite_code,
    };

    let app = Router::new()
        .nest("/api/v1", routes::router())
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
    tracing::info!("listening on {}", config.bind_addr);
    axum::serve(listener, app).await?;

    Ok(())
}
