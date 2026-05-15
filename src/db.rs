use sqlx::{postgres::PgPoolOptions, PgPool};

pub async fn connect(database_url: &str) -> anyhow::Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await
        .map_err(|e| anyhow::anyhow!("failed to connect to database: {}", e))
}
