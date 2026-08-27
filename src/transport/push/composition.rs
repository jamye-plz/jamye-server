//! Static worker composition for PostgreSQL push delivery through Expo.
//!
//! Task-9 owns this feature-local runtime. Task-12 owns final process
//! reachability and shutdown composition.

use std::{error::Error, fmt, future::Future, sync::Arc, time::Duration};

use uuid::Uuid;

use crate::{
    adapters::{
        postgres::{
            push::PostgresPushRepository, runtime_pool, transactions::SqlxTransactionManager,
        },
        push::expo::ExpoPushProvider,
    },
    application::push::{PushWorker, PushWorkerConfig, PushWorkerDependencies},
    config::{AppConfig, push::PushConfig},
};

pub struct WorkerRuntime {
    worker: PushWorker,
}

impl WorkerRuntime {
    pub fn from_worker(worker: PushWorker) -> Self {
        Self { worker }
    }

    pub fn poll_interval(&self) -> Duration {
        self.worker.poll_interval()
    }

    pub async fn run_until<F>(self, shutdown: F)
    where
        F: Future<Output = ()> + Send,
    {
        tokio::pin!(shutdown);
        loop {
            match self.worker.run_once().await {
                Ok(report) if report.claimed > 0 => {
                    tracing::info!(
                        target: "jamye_server",
                        event_kind = "expo_push_worker_batch",
                        claimed = report.claimed,
                        succeeded = report.succeeded,
                        retries = report.retries,
                        dead_lettered = report.dead_lettered,
                        invalid_destinations = report.invalid_destinations,
                        authorization_denied = report.authorization_denied,
                        stale_claims = report.stale_claims,
                        "Expo push worker batch completed"
                    );
                }
                Ok(_) => {}
                Err(_) => {
                    tracing::warn!(
                        target: "jamye_server",
                        event_kind = "expo_push_worker_poll_failed",
                        dependency = "postgres",
                        failure_kind = "worker_poll",
                        "Expo push worker poll failed"
                    );
                }
            }
            tokio::select! {
                () = &mut shutdown => break,
                () = tokio::time::sleep(self.worker.poll_interval()) => {}
            }
        }
    }
}

impl fmt::Debug for WorkerRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkerRuntime")
            .field("poll_interval", &self.worker.poll_interval())
            .finish()
    }
}

pub fn worker(
    app_config: &AppConfig,
    push_config: &PushConfig,
) -> Result<WorkerRuntime, WorkerCompositionError> {
    let pool = runtime_pool(app_config.database_url(), app_config.readiness_timeout())
        .map_err(|_| WorkerCompositionError::Postgres)?;
    let repository = Arc::new(PostgresPushRepository::new(pool.clone()));
    let transactions = Arc::new(SqlxTransactionManager::new(pool));
    let provider = Arc::new(
        ExpoPushProvider::new(
            push_config.endpoint(),
            push_config.access_token().map(str::to_owned),
        )
        .map_err(|_| WorkerCompositionError::Expo)?,
    );
    let worker = PushWorker::new(
        PushWorkerDependencies {
            transactions,
            repository: repository.clone(),
            preview_source: repository,
            provider,
        },
        PushWorkerConfig {
            claim_owner: format!("push-worker-{}", Uuid::new_v4()),
            batch_size: push_config.batch_size(),
            lease_duration: push_config.lease_duration(),
            provider_timeout: push_config.provider_timeout(),
            lease_safety_margin: push_config.lease_safety_margin(),
            retry_delay: push_config.retry_delay(),
            poll_interval: push_config.poll_interval(),
            max_attempts: push_config.max_attempts(),
        },
    )
    .map_err(|_| WorkerCompositionError::Worker)?;
    Ok(WorkerRuntime::from_worker(worker))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerCompositionError {
    Postgres,
    Expo,
    Worker,
}

impl fmt::Display for WorkerCompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("failed to initialize the Expo push worker")
    }
}

impl Error for WorkerCompositionError {}
