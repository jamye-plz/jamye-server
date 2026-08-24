//! PostgreSQL readiness probe.

use std::{error::Error, fmt, str::FromStr, time::Duration};

use sqlx::{
    ConnectOptions, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};

use crate::platform::readiness::{DependencyProbe, ProbeFuture, ProbeOutcome};

pub struct PostgresHealthProbe {
    pool: PgPool,
    timeout: Duration,
}

impl PostgresHealthProbe {
    pub fn connect_lazy(database_url: &str, timeout: Duration) -> Result<Self, PostgresInitError> {
        let options = PgConnectOptions::from_str(database_url)
            .map_err(|_| PostgresInitError)?
            .disable_statement_logging();
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(timeout)
            .connect_lazy_with(options);
        Ok(Self { pool, timeout })
    }
}

impl DependencyProbe for PostgresHealthProbe {
    fn check(&self) -> ProbeFuture<'_> {
        Box::pin(async move {
            let query = sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&self.pool);
            match tokio::time::timeout(self.timeout, query).await {
                Ok(Ok(1)) => ProbeOutcome::Reachable,
                Ok(Ok(_)) => {
                    log_failure("unexpected_result");
                    ProbeOutcome::Unreachable
                }
                Ok(Err(_)) => {
                    log_failure("query");
                    ProbeOutcome::Unreachable
                }
                Err(_) => {
                    log_failure("timeout");
                    ProbeOutcome::Unreachable
                }
            }
        })
    }
}

fn log_failure(failure_kind: &'static str) {
    tracing::warn!(
        dependency = "postgres",
        failure_kind,
        "dependency readiness check failed"
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PostgresInitError;

impl fmt::Display for PostgresInitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DATABASE_URL is not a valid PostgreSQL connection URL")
    }
}

impl Error for PostgresInitError {}
