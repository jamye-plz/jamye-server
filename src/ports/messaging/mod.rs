//! Persistence contract for message commands and versioned delta reads.

use std::{error::Error, fmt, future::Future, pin::Pin};

use uuid::Uuid;

use crate::{
    domain::messaging::{CanonicalMessage, EventPage, SendMessageCommand},
    ports::transactions::TransactionHandle,
};

pub type MessagingFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, MessagingRepositoryError>> + Send + 'a>>;

pub trait MessagingRepository: Send + Sync {
    fn send<'a>(
        &'a self,
        handle: &'a mut dyn TransactionHandle,
        command: &'a SendMessageCommand,
    ) -> MessagingFuture<'a, PersistMessageOutcome>;

    fn events(&self, query: DeltaQuery) -> MessagingFuture<'_, EventPage>;

    /// Resolves the persisted message's delivery topology from the authoritative
    /// database rows while the caller still owns the send transaction.
    fn delivery_context<'a>(
        &'a self,
        _handle: &'a mut dyn TransactionHandle,
        _message: &'a CanonicalMessage,
    ) -> MessagingFuture<'a, MessageDeliveryContext> {
        Box::pin(async { Err(MessagingRepositoryError::DatabaseUnavailable) })
    }

    /// Records the conversation-event/outbox row for a message that `send`
    /// already inserted but deliberately left event-less, so the caller can
    /// bind media (and thus finalize `message.media`) before the event's
    /// payload is written. Callers must invoke this exactly once per newly
    /// `Created` message, after media binding and before recording any
    /// notification for it.
    fn record_created_event<'a>(
        &'a self,
        _handle: &'a mut dyn TransactionHandle,
        _message: &'a CanonicalMessage,
    ) -> MessagingFuture<'a, PersistedMessage> {
        Box::pin(async { Err(MessagingRepositoryError::DatabaseUnavailable) })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersistMessageOutcome {
    /// The message row was just inserted. No conversation-event/outbox row
    /// exists for it yet -- the caller must bind media and then call
    /// `MessagingRepository::record_created_event` before this message is
    /// visible to any delta/realtime consumer.
    Created(CanonicalMessage),
    /// An idempotent retry matched an existing message row; its
    /// conversation-event was already recorded at original send time.
    Existing(PersistedMessage),
}

/// Repository-internal send result. It is deliberately distinct from the
/// public HTTP/application outcome because cross-feature callers need the
/// canonical conversation-event identity as well as the message projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistedMessage {
    message: CanonicalMessage,
    source_event_id: Uuid,
}

impl PersistedMessage {
    pub fn new(message: CanonicalMessage, source_event_id: Uuid) -> Self {
        Self {
            message,
            source_event_id,
        }
    }

    pub fn message(&self) -> &CanonicalMessage {
        &self.message
    }

    pub fn into_message(self) -> CanonicalMessage {
        self.message
    }

    pub fn source_event_id(&self) -> Uuid {
        self.source_event_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MessageDeliveryContext {
    Main,
    Topic {
        group_id: Uuid,
        topic_id: Uuid,
        sender_display_name: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeltaQuery {
    pub conversation_id: Uuid,
    pub user_id: Uuid,
    pub after: Option<i64>,
    pub limit: u32,
    pub projection: ContractProjection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContractProjection {
    Current,
    Previous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessagingRepositoryError {
    MembershipRequired,
    IdempotencyConflict,
    ContractUpgradeRequired,
    DatabaseUnavailable,
}

impl fmt::Display for MessagingRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("messaging persistence operation failed")
    }
}

impl Error for MessagingRepositoryError {}
