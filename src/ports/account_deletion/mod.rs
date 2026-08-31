//! Account-deletion persistence boundary.

use std::{fmt, future::Future, pin::Pin, time::Duration};

use time::OffsetDateTime;
use uuid::Uuid;

use crate::ports::{object_storage::ObjectStorageProviderError, transactions::TransactionHandle};

/// The fixed public projection used by retained content after account deletion.
pub const ANONYMOUS_AUTHOR_NICKNAME: &str = "탈퇴한 사용자";

pub type AccountDeletionRepositoryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, AccountDeletionRepositoryError>> + Send + 'a>>;

/// Defines the current account targeted by an account-deletion operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccountDeletionCommand {
    pub user_id: Uuid,
}

/// A stable membership enumeration collected after the ownership fence passes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountDeletionPreparation {
    pub memberships: Vec<AccountDeletionMembership>,
}

/// One membership that the deletion service must remove through `GroupsService`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccountDeletionMembership {
    pub membership_id: Uuid,
    pub group_id: Uuid,
}

/// Publicly reportable counts from one successful account deletion.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AccountDeletionReport {
    pub memberships_removed: u64,
    pub cleanup_intents_enqueued: u64,
}

pub trait AccountDeletionRepository: Send + Sync {
    fn prepare_deletion<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        user_id: Uuid,
    ) -> AccountDeletionRepositoryFuture<'a, AccountDeletionPreparation>;

    fn finalize_deletion<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        user_id: Uuid,
    ) -> AccountDeletionRepositoryFuture<'a, AccountDeletionReport>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountDeletionRepositoryError {
    GroupOwnershipTransferRequired,
    AccountNotFound,
    InvalidData,
    Unavailable,
}

impl fmt::Display for AccountDeletionRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("account-deletion persistence operation failed")
    }
}

impl std::error::Error for AccountDeletionRepositoryError {}

/// Durable account-object cleanup persistence future.
pub type AccountObjectDeletionRepositoryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, AccountObjectDeletionRepositoryError>> + Send + 'a>>;

/// External delete call for one already-authorized account cleanup object.
pub type AccountObjectDeletionProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), ObjectStorageProviderError>> + Send + 'a>>;

/// DB-time claim request for account-object deletion cleanup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountObjectDeletionClaimRequest {
    pub claim_owner: String,
    pub batch_size: u32,
    pub lease_duration: Duration,
}

/// Generation-fenced durable cleanup claim.
#[derive(Clone, Eq, PartialEq)]
pub struct AccountObjectDeletionClaim {
    pub intent_id: Uuid,
    object_key: String,
    pub claim_owner: String,
    pub claim_generation: i64,
    pub claim_expires_at: OffsetDateTime,
    pub attempt_count: u32,
}

impl AccountObjectDeletionClaim {
    pub fn new(
        intent_id: Uuid,
        object_key: String,
        claim_owner: String,
        claim_generation: i64,
        claim_expires_at: OffsetDateTime,
        attempt_count: u32,
    ) -> Self {
        Self {
            intent_id,
            object_key,
            claim_owner,
            claim_generation,
            claim_expires_at,
            attempt_count,
        }
    }

    pub fn object_key(&self) -> &str {
        &self.object_key
    }
}

impl fmt::Debug for AccountObjectDeletionClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccountObjectDeletionClaim")
            .field("intent_id", &self.intent_id)
            .field("object_key", &"[REDACTED]")
            .field("claim_owner", &self.claim_owner)
            .field("claim_generation", &self.claim_generation)
            .field("claim_expires_at", &self.claim_expires_at)
            .field("attempt_count", &self.attempt_count)
            .finish()
    }
}

/// Durable state transitions for at-least-once account object cleanup.
pub trait AccountObjectDeletionRepository: Send + Sync {
    fn claim_object_deletions(
        &self,
        request: AccountObjectDeletionClaimRequest,
    ) -> AccountObjectDeletionRepositoryFuture<'_, Vec<AccountObjectDeletionClaim>>;

    fn mark_object_deleted<'a>(
        &'a self,
        claim: &'a AccountObjectDeletionClaim,
    ) -> AccountObjectDeletionRepositoryFuture<'a, bool>;

    fn record_object_deletion_failure<'a>(
        &'a self,
        claim: &'a AccountObjectDeletionClaim,
        code: AccountObjectDeletionFailureCode,
        retry_delay: Duration,
        max_attempts: u32,
    ) -> AccountObjectDeletionRepositoryFuture<'a, AccountObjectDeletionFailureDisposition>;
}

/// Feature-local provider boundary: this intentionally does not extend Task-8 media storage.
pub trait AccountObjectDeletionProvider: Send + Sync {
    fn delete_object<'a>(&'a self, object_key: &'a str) -> AccountObjectDeletionProviderFuture<'a>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountObjectDeletionFailureCode {
    AccessDenied,
    Unavailable,
    UnexpectedResponse,
    Timeout,
}

impl AccountObjectDeletionFailureCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AccessDenied => "access_denied",
            Self::Unavailable => "unavailable",
            Self::UnexpectedResponse => "unexpected_response",
            Self::Timeout => "timeout",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountObjectDeletionFailureDisposition {
    RetryScheduled,
    Failed,
    DeadLettered,
    StaleClaim,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountObjectDeletionRepositoryError {
    InvalidData,
    Unavailable,
}

impl fmt::Display for AccountObjectDeletionRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("account object-deletion persistence operation failed")
    }
}

impl std::error::Error for AccountObjectDeletionRepositoryError {}
