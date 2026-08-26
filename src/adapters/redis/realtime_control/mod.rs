//! Versioned internal Redis controls and their durable control-outbox runner.

use std::{error::Error, fmt, time::Duration};

use futures_util::{StreamExt, future::join_all};
use redis::{Client, aio::PubSub};

use crate::{
    adapters::postgres::realtime_revocations::{
        ClaimedControlIntent, ControlClaimRequest, ControlFailureDisposition,
        PostgresRealtimeRevocations,
    },
    application::realtime::membership_revocation::RealtimeControlIntent,
};

const CONTROL_CHANNEL: &str = "jamye:realtime:control:v1";

#[derive(Clone)]
pub struct RedisRealtimeControl {
    client: Client,
}

impl RedisRealtimeControl {
    pub fn new(redis_url: &str) -> Result<Self, RealtimeControlError> {
        Client::open(redis_url)
            .map(|client| Self { client })
            .map_err(|_| redis_error("configure"))
    }

    pub async fn subscriber(&self) -> Result<RedisControlSubscriber, RealtimeControlError> {
        let mut pubsub = self
            .client
            .get_async_pubsub()
            .await
            .map_err(|_| redis_error("subscriber_connect"))?;
        pubsub
            .subscribe(CONTROL_CHANNEL)
            .await
            .map_err(|_| redis_error("subscriber_subscribe"))?;
        Ok(RedisControlSubscriber { pubsub })
    }

    pub async fn publish(
        &self,
        intent: &RealtimeControlIntent,
    ) -> Result<(), RealtimeControlError> {
        let payload =
            serde_json::to_string(intent).map_err(|_| RealtimeControlError::InvalidData)?;
        let mut connection = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|_| redis_error("publisher_connect"))?;
        redis::cmd("PUBLISH")
            .arg(CONTROL_CHANNEL)
            .arg(payload)
            .query_async::<i64>(&mut connection)
            .await
            .map(|_| ())
            .map_err(|_| redis_error("publish"))
    }
}

pub struct RedisControlSubscriber {
    pubsub: PubSub,
}

impl RedisControlSubscriber {
    pub async fn next_control(
        &mut self,
    ) -> Result<Option<RealtimeControlIntent>, RealtimeControlError> {
        let mut stream = self.pubsub.on_message();
        let Some(message) = stream.next().await else {
            return Ok(None);
        };
        let payload = message
            .get_payload::<String>()
            .map_err(|_| RealtimeControlError::InvalidData)?;
        serde_json::from_str(&payload)
            .map(Some)
            .map_err(|_| RealtimeControlError::InvalidData)
    }
}

#[derive(Clone)]
pub struct RealtimeControlWorker {
    repository: PostgresRealtimeRevocations,
    publisher: RedisRealtimeControl,
    config: RealtimeControlWorkerConfig,
}

impl RealtimeControlWorker {
    pub fn new(
        repository: PostgresRealtimeRevocations,
        publisher: RedisRealtimeControl,
        config: RealtimeControlWorkerConfig,
    ) -> Result<Self, RealtimeControlWorkerError> {
        config.validate()?;
        Ok(Self {
            repository,
            publisher,
            config,
        })
    }

    pub async fn run_once(&self) -> Result<RealtimeControlWorkerReport, RealtimeControlWorkerError> {
        let claims = self
            .repository
            .claim_controls(&ControlClaimRequest {
                claim_owner: self.config.claim_owner.clone(),
                batch_size: self.config.batch_size,
                lease_duration: self.config.lease_duration,
            })
            .await
            .map_err(|_| RealtimeControlWorkerError::RepositoryUnavailable)?;
        let mut report = RealtimeControlWorkerReport {
            claimed: claims.len(),
            ..RealtimeControlWorkerReport::default()
        };
        let outcomes = join_all(
            claims
                .into_iter()
                .map(|claim| self.process_claim(claim)),
        )
        .await;
        for outcome in outcomes {
            report.merge(outcome?);
        }
        Ok(report)
    }

    async fn process_claim(
        &self,
        claim: ClaimedControlIntent,
    ) -> Result<RealtimeControlWorkerReport, RealtimeControlWorkerError> {
        let mut report = RealtimeControlWorkerReport::default();
        match tokio::time::timeout(
            self.config.publish_timeout,
            self.publisher.publish(&claim.intent),
        )
        .await
        {
            Ok(Ok(())) => {
                if self
                    .repository
                    .mark_published(&claim)
                    .await
                    .map_err(|_| RealtimeControlWorkerError::RepositoryUnavailable)?
                {
                    report.published += 1;
                } else {
                    report.stale_claims += 1;
                }
            }
            Ok(Err(_)) => {
                self.record_failure(&claim, "redis_unavailable", &mut report)
                    .await?;
            }
            Err(_) => {
                self.record_failure(&claim, "publish_timeout", &mut report)
                    .await?;
            }
        }
        Ok(report)
    }

    async fn record_failure(
        &self,
        claim: &ClaimedControlIntent,
        code: &'static str,
        report: &mut RealtimeControlWorkerReport,
    ) -> Result<(), RealtimeControlWorkerError> {
        match self
            .repository
            .record_failure(
                claim,
                code,
                self.config.retry_delay,
                self.config.max_attempts,
            )
            .await
            .map_err(|_| RealtimeControlWorkerError::RepositoryUnavailable)?
        {
            ControlFailureDisposition::RetryScheduled => report.retries += 1,
            ControlFailureDisposition::DeadLettered => {
                report.dead_lettered += 1;
                tracing::error!(
                    outbox_event_id = %claim.id,
                    failure_code = code,
                    "realtime control reached the terminal dead-letter state"
                );
            }
            ControlFailureDisposition::StaleClaim => report.stale_claims += 1,
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RealtimeControlWorkerConfig {
    pub claim_owner: String,
    pub batch_size: u32,
    pub lease_duration: Duration,
    pub publish_timeout: Duration,
    pub lease_safety_margin: Duration,
    pub retry_delay: Duration,
    pub max_attempts: u32,
}

impl RealtimeControlWorkerConfig {
    fn validate(&self) -> Result<(), RealtimeControlWorkerError> {
        let publish_budget = self
            .publish_timeout
            .checked_add(self.lease_safety_margin)
            .ok_or(RealtimeControlWorkerError::InvalidConfiguration)?;
        if self.claim_owner.is_empty()
            || self.claim_owner.len() > 128
            || self.batch_size == 0
            || self.max_attempts == 0
            || self.lease_duration <= publish_budget
            || self.retry_delay.is_zero()
        {
            return Err(RealtimeControlWorkerError::InvalidConfiguration);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RealtimeControlWorkerReport {
    pub claimed: usize,
    pub published: usize,
    pub retries: usize,
    pub dead_lettered: usize,
    pub stale_claims: usize,
}

impl RealtimeControlWorkerReport {
    fn merge(&mut self, other: Self) {
        self.published += other.published;
        self.retries += other.retries;
        self.dead_lettered += other.dead_lettered;
        self.stale_claims += other.stale_claims;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RealtimeControlError {
    Unavailable,
    InvalidData,
}

impl fmt::Display for RealtimeControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Redis realtime control operation failed")
    }
}

impl Error for RealtimeControlError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RealtimeControlWorkerError {
    InvalidConfiguration,
    RepositoryUnavailable,
}

impl fmt::Display for RealtimeControlWorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("realtime control worker operation failed")
    }
}

impl Error for RealtimeControlWorkerError {}

fn redis_error(operation: &'static str) -> RealtimeControlError {
    tracing::warn!(
        dependency = "redis",
        failure_kind = "realtime_control",
        operation,
        "Redis realtime control operation failed"
    );
    RealtimeControlError::Unavailable
}
