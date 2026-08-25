use std::{error::Error, fmt, sync::Arc, time::Duration};

use futures_util::future::join_all;

use crate::ports::realtime::{
    ClaimedOutboxEvent, FailureDisposition, OutboxClaimRequest, OutboxRepository,
    PublishFailureCode, RealtimeEventPublisher, RealtimePortError,
};

#[derive(Clone)]
pub struct OutboxWorker {
    repository: Arc<dyn OutboxRepository>,
    publisher: Arc<dyn RealtimeEventPublisher>,
    config: OutboxWorkerConfig,
}

impl OutboxWorker {
    pub fn new(
        repository: Arc<dyn OutboxRepository>,
        publisher: Arc<dyn RealtimeEventPublisher>,
        config: OutboxWorkerConfig,
    ) -> Result<Self, OutboxWorkerError> {
        config.validate()?;
        Ok(Self {
            repository,
            publisher,
            config,
        })
    }

    pub async fn run_once(&self) -> Result<WorkerRunReport, OutboxWorkerError> {
        let claims = self
            .repository
            .claim(OutboxClaimRequest {
                claim_owner: self.config.claim_owner.clone(),
                batch_size: self.config.batch_size,
                lease_duration: self.config.lease_duration,
            })
            .await
            .map_err(|_| OutboxWorkerError::RepositoryUnavailable)?;
        let mut report = WorkerRunReport {
            claimed: claims.len(),
            ..WorkerRunReport::default()
        };
        let outcomes = join_all(claims.into_iter().map(|claim| self.process_claim(claim))).await;
        for outcome in outcomes {
            report.merge(outcome?);
        }
        Ok(report)
    }

    async fn process_claim(
        &self,
        claim: ClaimedOutboxEvent,
    ) -> Result<WorkerRunReport, OutboxWorkerError> {
        let mut report = WorkerRunReport::default();
        let publish_result =
            tokio::time::timeout(self.config.publish_timeout, self.publisher.publish(&claim)).await;
        match publish_result {
            Ok(Ok(())) => {
                if self
                    .repository
                    .mark_published(&claim)
                    .await
                    .map_err(|_| OutboxWorkerError::RepositoryUnavailable)?
                {
                    report.published = 1;
                } else {
                    report.stale_claims = 1;
                }
            }
            Ok(Err(_)) => {
                self.record_failure(&claim, PublishFailureCode::RedisUnavailable, &mut report)
                    .await?;
            }
            Err(_) => {
                self.record_failure(&claim, PublishFailureCode::PublishTimeout, &mut report)
                    .await?;
            }
        }
        Ok(report)
    }

    async fn record_failure(
        &self,
        claim: &crate::ports::realtime::ClaimedOutboxEvent,
        code: PublishFailureCode,
        report: &mut WorkerRunReport,
    ) -> Result<(), OutboxWorkerError> {
        match self
            .repository
            .record_failure(
                claim,
                code,
                self.config.retry_delay,
                self.config.max_attempts,
            )
            .await
            .map_err(|_| OutboxWorkerError::RepositoryUnavailable)?
        {
            FailureDisposition::RetryScheduled => report.retries += 1,
            FailureDisposition::DeadLettered => {
                report.dead_lettered += 1;
                tracing::error!(
                    outbox_event_id = %claim.id,
                    failure_code = code.as_str(),
                    "outbox event reached the terminal dead-letter state"
                );
            }
            FailureDisposition::StaleClaim => report.stale_claims += 1,
        }
        Ok(())
    }

    pub fn poll_interval(&self) -> Duration {
        self.config.poll_interval
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutboxWorkerConfig {
    pub claim_owner: String,
    pub batch_size: u32,
    pub lease_duration: Duration,
    pub publish_timeout: Duration,
    pub lease_safety_margin: Duration,
    pub retry_delay: Duration,
    pub poll_interval: Duration,
    pub max_attempts: u32,
}

impl OutboxWorkerConfig {
    fn validate(&self) -> Result<(), OutboxWorkerError> {
        let publish_budget = self
            .publish_timeout
            .checked_add(self.lease_safety_margin)
            .ok_or(OutboxWorkerError::InvalidConfiguration)?;
        if self.claim_owner.is_empty()
            || self.claim_owner.len() > 128
            || self.batch_size == 0
            || self.max_attempts == 0
            || self.lease_duration <= publish_budget
            || self.retry_delay.is_zero()
            || self.poll_interval.is_zero()
        {
            return Err(OutboxWorkerError::InvalidConfiguration);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WorkerRunReport {
    pub claimed: usize,
    pub published: usize,
    pub retries: usize,
    pub dead_lettered: usize,
    pub stale_claims: usize,
}

impl WorkerRunReport {
    fn merge(&mut self, other: Self) {
        self.published += other.published;
        self.retries += other.retries;
        self.dead_lettered += other.dead_lettered;
        self.stale_claims += other.stale_claims;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutboxWorkerError {
    InvalidConfiguration,
    RepositoryUnavailable,
}

impl fmt::Display for OutboxWorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("outbox worker operation failed")
    }
}

impl Error for OutboxWorkerError {}

impl From<RealtimePortError> for OutboxWorkerError {
    fn from(_: RealtimePortError) -> Self {
        Self::RepositoryUnavailable
    }
}
