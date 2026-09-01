use std::{
    error::Error,
    fs, io,
    sync::{Mutex, MutexGuard},
};

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

fn lock_test_mutex<'a, T>(mutex: &'a Mutex<T>, name: &str) -> MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(_) => panic!("{name} mutex is poisoned"),
    }
}

fn logging_interest_sentinel() -> tracing::Dispatch {
    // Keep two scoped dispatchers registered while a private writer is active.
    // This prevents tracing-core's single-dispatch fast path from caching a new
    // callsite as disabled when a parallel test thread observes it first.
    tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default())
}

#[path = "media/access_adapters.rs"]
mod access_adapters;
#[path = "media/access_http.rs"]
mod access_http;
#[path = "media/access_orchestration.rs"]
mod access_orchestration;
#[path = "media/access_policy.rs"]
mod access_policy;
#[path = "media/bucket_lifecycle.rs"]
mod bucket_lifecycle;
#[path = "media/config.rs"]
mod config;
#[path = "media/contract.rs"]
mod contract;
#[path = "media/finalize_adapters.rs"]
mod finalize_adapters;
#[path = "media/finalize_orchestration.rs"]
mod finalize_orchestration;
#[path = "media/finalize_policy.rs"]
mod finalize_policy;
#[path = "support/logging.rs"]
mod logging_support;
#[path = "media/md3_http.rs"]
mod md3_http;
#[path = "media/message_binding_adapters.rs"]
mod message_binding_adapters;
#[path = "media/message_policy.rs"]
mod message_policy;
#[cfg(feature = "dev-fixtures")]
#[allow(
    dead_code,
    reason = "task-8 reuses a subset; remaining helpers are exercised by tests/messaging.rs"
)]
#[path = "messaging/helpers.rs"]
mod messaging_helpers;
#[path = "media/migration.rs"]
mod migration;
#[path = "media/minio_boundary.rs"]
mod minio_boundary;
#[path = "media/policy.rs"]
mod policy;
#[path = "support/postgres.rs"]
mod postgres_support;
#[path = "media/read_projections.rs"]
mod read_projections;
#[cfg(feature = "dev-fixtures")]
#[path = "media/resilience.rs"]
mod resilience;
#[path = "media/s3_bucket_backend.rs"]
mod s3_bucket_backend;
#[path = "media/upload_finalize_http.rs"]
mod upload_finalize_http;
#[path = "media/upload_intent.rs"]
mod upload_intent;
#[path = "media/upload_intent_adapters.rs"]
mod upload_intent_adapters;

const MEDIA_MIGRATION: &str = "migrations/0006_media.sql";

#[test]
fn production_media_surface_is_absent_before_task_8() -> TestResult {
    fs::read_to_string(MEDIA_MIGRATION)
        .map(|_| ())
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                io::Error::other(format!(
                    "RED: {MEDIA_MIGRATION} is absent; task-8 must add D11=B API-owned private-bucket lifecycle, authorized upload/finalize, ordered message media, and presigned access"
                ))
                .into()
            } else {
                error.into()
            }
        })
}
