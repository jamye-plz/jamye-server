//! Static worker composition for PostgreSQL outbox delivery through Redis.

use std::{error::Error, fmt, future::Future, sync::Arc, time::Duration};

use uuid::Uuid;

use crate::{
    adapters::{
        postgres::{realtime::PostgresRealtimeRepository, runtime_pool},
        redis::realtime::RedisRealtimeAdapter,
    },
    application::realtime::{OutboxWorker, OutboxWorkerConfig},
    config::AppConfig,
};

pub struct WorkerRuntime {
    worker: OutboxWorker,
}

impl WorkerRuntime {
    pub async fn run_until<F>(self, shutdown: F)
    where
        F: Future<Output = ()> + Send,
    {
        tokio::pin!(shutdown);
        loop {
            match self.worker.run_once().await {
                Ok(report) if report.claimed > 0 => {
                    tracing::info!(
                        claimed = report.claimed,
                        published = report.published,
                        retries = report.retries,
                        dead_lettered = report.dead_lettered,
                        stale_claims = report.stale_claims,
                        "outbox worker batch completed"
                    );
                }
                Ok(_) => {}
                Err(_) => {
                    tracing::warn!(
                        dependency = "postgres",
                        failure_kind = "worker_poll",
                        "outbox worker poll failed"
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

pub fn worker(config: &AppConfig) -> Result<WorkerRuntime, WorkerCompositionError> {
    let pool = runtime_pool(config.database_url(), config.readiness_timeout())
        .map_err(|_| WorkerCompositionError::Postgres)?;
    let redis_url = config
        .redis_url()
        .ok_or(WorkerCompositionError::RedisNotConfigured)?;
    let redis =
        Arc::new(RedisRealtimeAdapter::new(redis_url).map_err(|_| WorkerCompositionError::Redis)?);
    let repository = Arc::new(PostgresRealtimeRepository::new(pool));
    let worker = OutboxWorker::new(
        repository,
        redis,
        OutboxWorkerConfig {
            claim_owner: format!("worker-{}", Uuid::new_v4()),
            batch_size: 50,
            lease_duration: Duration::from_secs(15),
            publish_timeout: Duration::from_secs(2),
            lease_safety_margin: Duration::from_secs(1),
            retry_delay: Duration::from_secs(1),
            poll_interval: Duration::from_millis(250),
            max_attempts: 8,
        },
    )
    .map_err(|_| WorkerCompositionError::Worker)?;
    Ok(WorkerRuntime { worker })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerCompositionError {
    Postgres,
    RedisNotConfigured,
    Redis,
    Worker,
}

impl fmt::Display for WorkerCompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("failed to initialize the realtime worker")
    }
}

impl Error for WorkerCompositionError {}
