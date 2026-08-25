//! Replaceable persistence, delivery, authorization, clock, and credential ports for realtime.

use std::{error::Error, fmt, future::Future, pin::Pin, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

pub type RealtimeFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, RealtimePortError>> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RealtimePortError {
    Unavailable,
    InvalidData,
}

impl fmt::Display for RealtimePortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("realtime dependency operation failed")
    }
}

impl Error for RealtimePortError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutboxClaimRequest {
    pub claim_owner: String,
    pub batch_size: u32,
    pub lease_duration: Duration,
}

#[derive(Clone)]
pub struct ClaimedOutboxEvent {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub event_id: Uuid,
    pub payload: Value,
    pub claim_owner: String,
    pub claim_generation: i64,
    pub claim_expires_at: OffsetDateTime,
    pub attempt_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublishFailureCode {
    RedisUnavailable,
    PublishTimeout,
}

impl PublishFailureCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RedisUnavailable => "redis_unavailable",
            Self::PublishTimeout => "publish_timeout",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureDisposition {
    RetryScheduled,
    DeadLettered,
    StaleClaim,
}

pub trait OutboxRepository: Send + Sync {
    fn claim(&self, request: OutboxClaimRequest) -> RealtimeFuture<'_, Vec<ClaimedOutboxEvent>>;

    fn mark_published<'a>(&'a self, claim: &'a ClaimedOutboxEvent) -> RealtimeFuture<'a, bool>;

    fn record_failure<'a>(
        &'a self,
        claim: &'a ClaimedOutboxEvent,
        code: PublishFailureCode,
        retry_delay: Duration,
        max_attempts: u32,
    ) -> RealtimeFuture<'a, FailureDisposition>;
}

pub trait RealtimeEventPublisher: Send + Sync {
    fn publish<'a>(&'a self, event: &'a ClaimedOutboxEvent) -> RealtimeFuture<'a, ()>;
}

#[derive(Clone, Eq, PartialEq)]
pub struct TicketSecret(String);

impl TicketSecret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for TicketSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct TicketDigest(String);

impl TicketDigest {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose_for_storage(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for TicketDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TicketCredential {
    pub secret: TicketSecret,
    pub digest: TicketDigest,
}

pub trait TicketCredentialSource: Send + Sync {
    fn generate(&self) -> Result<TicketCredential, RealtimePortError>;

    fn digest(&self, raw_ticket: &str) -> Result<TicketDigest, RealtimePortError>;
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RealtimeTicketRecord {
    pub user_id: Uuid,
    pub session_id: Uuid,
    pub contract_version: String,
    #[serde(with = "time::serde::rfc3339")]
    pub access_token_expires_at: OffsetDateTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TicketPutOutcome {
    Stored,
    Collision,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TicketConsumeOutcome {
    Found(RealtimeTicketRecord),
    Missing,
}

pub trait RealtimeTicketStore: Send + Sync {
    fn put<'a>(
        &'a self,
        digest: &'a TicketDigest,
        record: &'a RealtimeTicketRecord,
        ttl: Duration,
    ) -> RealtimeFuture<'a, TicketPutOutcome>;

    fn consume<'a>(&'a self, digest: &'a TicketDigest) -> RealtimeFuture<'a, TicketConsumeOutcome>;
}

pub trait RealtimeClock: Send + Sync {
    fn now(&self) -> OffsetDateTime;
}

pub trait ConversationAuthorizer: Send + Sync {
    fn is_authorized(&self, user_id: Uuid, conversation_id: Uuid) -> RealtimeFuture<'_, bool>;
}
