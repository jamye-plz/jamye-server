use std::error::Error;

#[allow(
    dead_code,
    reason = "auth logging tests reuse only the logging subset of the full auth harness"
)]
#[path = "auth/helpers.rs"]
mod auth_helpers;
#[path = "auth/logging.rs"]
mod logging;
#[path = "support/postgres.rs"]
mod postgres_support;

pub type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;
