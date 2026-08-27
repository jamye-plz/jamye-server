use std::{
    error::Error,
    fs, io,
    sync::{Mutex, MutexGuard},
};

pub type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

fn lock_test_mutex<'a, T>(mutex: &'a Mutex<T>, name: &str) -> MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(_) => panic!("{name} mutex is poisoned"),
    }
}

#[path = "notifications/contract.rs"]
mod contract;
#[path = "notifications/delivery_adapters.rs"]
mod delivery_adapters;
#[path = "notifications/delivery_lifecycle.rs"]
mod delivery_lifecycle;
#[path = "notifications/delivery_worker.rs"]
mod delivery_worker;
#[path = "notifications/event_operations.rs"]
mod event_operations;
#[path = "notifications/expo_adapter.rs"]
mod expo_adapter;
#[path = "notifications/history_adapters.rs"]
mod history_adapters;
#[path = "notifications/history_orchestration.rs"]
mod history_orchestration;
#[path = "notifications/http.rs"]
mod http;
#[path = "notifications/installation_adapters.rs"]
mod installation_adapters;
#[path = "notifications/installation_orchestration.rs"]
mod installation_orchestration;
#[path = "support/logging.rs"]
mod logging_support;
#[path = "notifications/migration.rs"]
mod migration;
#[path = "support/postgres.rs"]
mod postgres_support;
#[path = "notifications/privacy_mutations.rs"]
mod privacy_mutations;
#[path = "notifications/push_config.rs"]
mod push_config;
#[path = "notifications/push_runtime.rs"]
mod push_runtime;
#[path = "notifications/send_authorization.rs"]
mod send_authorization;

const NOTIFICATIONS_MIGRATION: &str = "migrations/0007_notifications_push.sql";

#[test]
fn production_notifications_surface_is_absent_before_task_9() -> TestResult {
    fs::read_to_string(NOTIFICATIONS_MIGRATION)
        .map(|_| ())
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                io::Error::other(format!(
                    "RED: {NOTIFICATIONS_MIGRATION} is absent; task-9 must add D9=A structured notification history, Expo-only installation ownership, durable source-event push occurrences, and privacy-fenced delivery"
                ))
                .into()
            } else {
                error.into()
            }
        })
}
