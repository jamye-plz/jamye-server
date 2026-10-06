//! Server task-19 avatar upload (U4 start, U5 finalize, U6 public read).

use std::error::Error;

pub type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[path = "avatar/config.rs"]
mod config;
#[path = "avatar/finalize.rs"]
mod finalize;
#[path = "avatar/migration.rs"]
mod migration;
#[path = "support/postgres.rs"]
mod postgres_support;
#[path = "avatar/public_read.rs"]
mod public_read;
#[path = "avatar/support.rs"]
mod support;
#[path = "avatar/upload_intent.rs"]
mod upload_intent;
