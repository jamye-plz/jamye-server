use std::{error::Error, fs};

pub type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[path = "account_deletion/cleanup.rs"]
mod cleanup;
#[path = "account_deletion/http.rs"]
mod http;
#[path = "account_deletion/migration.rs"]
mod migration;
#[path = "support/postgres.rs"]
mod postgres_support;
#[path = "account_deletion/push_barriers.rs"]
mod push_barriers;
#[allow(
    dead_code,
    reason = "the shared Task-9 topology exposes fields used by its owning test target"
)]
#[path = "notifications/send_authorization/helpers.rs"]
mod send_topology_support;
#[path = "account_deletion/support.rs"]
mod support;
#[path = "account_deletion/transition.rs"]
mod transition;

const TASK_11_TRANSITION_SURFACES: &[&str] = &[
    "migrations/0008_account_deletion.sql",
    "src/application/account_deletion/mod.rs",
    "src/ports/account_deletion/mod.rs",
    "src/adapters/postgres/account_deletion/mod.rs",
];

#[test]
fn production_account_deletion_transition_surface_is_registered() -> TestResult {
    for path in TASK_11_TRANSITION_SURFACES {
        if !fs::metadata(path)?.is_file() {
            return Err(format!("missing Task-11 Sprint-2 transition surface: {path}").into());
        }
    }
    Ok(())
}
