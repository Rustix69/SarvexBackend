use anyhow::{Context, Result};
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::env;

pub fn database_url() -> Result<String> {
    env::var("DATABASE_URL")
        .or_else(|_| env::var("POSTGRES_URL"))
        .or_else(|_| {
            let user = env::var("POSTGRES_USER").unwrap_or_else(|_| "sarvex".to_owned());
            let password = env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "sarvex".to_owned());
            let host = env::var("POSTGRES_HOST").unwrap_or_else(|_| "127.0.0.1".to_owned());
            let port = env::var("POSTGRES_PORT").unwrap_or_else(|_| "5432".to_owned());
            let db = env::var("POSTGRES_DB").unwrap_or_else(|_| "sarvex".to_owned());
            Ok(format!("postgres://{user}:{password}@{host}:{port}/{db}"))
        })
        .context("database configuration is unavailable")
}

pub async fn connect() -> Result<PgPool> {
    let url = database_url()?;
    PgPoolOptions::new()
        .max_connections(env::var("DB_MAX_CONNECTIONS").ok().and_then(|v| v.parse().ok()).unwrap_or(10))
        .connect(&url)
        .await
        .with_context(|| format!("failed to connect to PostgreSQL at {url}"))
}
