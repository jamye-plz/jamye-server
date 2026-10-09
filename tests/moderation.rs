//! Server task-20b UGC moderation: content filter (R8), reports (R1), blocks (B1-B3),
//! suspension, operator alert, push suppression, purge interaction and migration 0020.

use std::error::Error;

pub type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[path = "moderation/blocks.rs"]
mod blocks;
#[path = "moderation/config.rs"]
mod config;
#[path = "moderation/filter.rs"]
mod filter;
#[path = "moderation/messages.rs"]
mod messages;
#[path = "moderation/migration.rs"]
mod migration;
#[path = "support/postgres.rs"]
mod postgres_support;
#[path = "moderation/purge.rs"]
mod purge;
#[path = "moderation/push.rs"]
mod push;
#[path = "moderation/realtime.rs"]
mod realtime;
#[path = "moderation/reports.rs"]
mod reports;
#[path = "moderation/support.rs"]
mod support;
#[path = "moderation/suspension.rs"]
mod suspension;
