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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersistMessageOutcome {
    Created(CanonicalMessage),
    Existing(CanonicalMessage),
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
