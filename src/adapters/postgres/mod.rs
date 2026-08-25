//! PostgreSQL adapters.

use std::{str::FromStr, time::Duration};

use sqlx::{
    ConnectOptions, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};

#[cfg(feature = "dev-fixtures")]
pub mod dev_fixtures;
pub mod health;
pub mod messaging;
pub mod realtime;
pub mod transactions;

pub fn runtime_pool(
    database_url: &str,
    acquire_timeout: Duration,
) -> Result<PgPool, health::PostgresInitError> {
    let options = PgConnectOptions::from_str(database_url)
        .map_err(|_| health::PostgresInitError)?
        .disable_statement_logging();
    Ok(PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(acquire_timeout)
        .connect_lazy_with(options))
}
