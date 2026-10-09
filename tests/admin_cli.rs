//! Server task-20c admin CLI: `reports list|show|dismiss|resolve|purge`, `messages delete`,
//! `users suspend|unsuspend` and `accounts delete`, against real PostgreSQL, plus the binary's
//! exit codes, streams and absence of a listener.

use std::error::Error;

pub type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[path = "admin_cli/accounts.rs"]
mod accounts;
#[path = "admin_cli/binary.rs"]
mod binary;
#[path = "admin_cli/harness.rs"]
mod harness;
#[path = "admin_cli/messages.rs"]
mod messages;
#[path = "support/postgres.rs"]
mod postgres_support;
#[path = "admin_cli/reports.rs"]
mod reports;
// Shared fixtures (users, groups, messages, installations); its HTTP harness stays unused here.
#[path = "moderation/support.rs"]
mod support;
#[path = "admin_cli/usage.rs"]
mod usage;
#[path = "admin_cli/users.rs"]
mod users;
