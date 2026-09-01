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
        _message: &'a PersistedMessage,
    ) -> MessagingFuture<'a, MessageDeliveryContext> {
        Box::pin(async { Err(MessagingRepositoryError::DatabaseUnavailable) })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersistMessageOutcome {
    Created(PersistedMessage),
    Existing(PersistedMessage),
}

impl PersistMessageOutcome {
    pub fn into_persisted(self) -> PersistedMessage {
        match self {
            Self::Created(message) | Self::Existing(message) => message,
        }
    }
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
