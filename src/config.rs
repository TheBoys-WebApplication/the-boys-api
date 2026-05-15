use anyhow::{Context, Result};

pub struct Config {
    pub database_url: String,
    pub jwt_secret: String,
    pub bind_addr: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            database_url: std::env::var("DATABASE_URL").context("DATABASE_URL not set")?,
            jwt_secret: std::env::var("JWT_SECRET").context("JWT_SECRET not set")?,
            bind_addr: std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".into()),
        })
    }
}
