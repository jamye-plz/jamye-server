use std::error::Error;

#[allow(
    dead_code,
    reason = "auth logging tests reuse only the logging subset of the full auth harness"
)]
#[path = "auth/helpers.rs"]
mod auth_helpers;
#[path = "auth/logging.rs"]
mod logging;
#[allow(
    dead_code,
    reason = "auth logging tests read raw log snapshots and do not use parsed_lines"
)]
#[path = "support/logging.rs"]
mod logging_support;
#[path = "support/postgres.rs"]
mod postgres_support;

pub type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;
