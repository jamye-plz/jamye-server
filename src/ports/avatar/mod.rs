//! Authoritative PostgreSQL avatar-upload persistence boundary.

use std::{fmt, future::Future, pin::Pin, time::Duration};

use time::OffsetDateTime;
use uuid::Uuid;

use crate::ports::{auth::UserProfile, transactions::TransactionHandle};

pub type AvatarRepositoryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, AvatarRepositoryError>> + Send + 'a>>;

pub trait AvatarRepository: Send + Sync {
    /// Release the actor's previous pending row (queueing its object for deletion) and
    /// insert the new pending row, all inside the caller-owned transaction.
    fn create_intent<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a CreateAvatarIntentCommand,
    ) -> AvatarRepositoryFuture<'a, AvatarIntentRecord>;

    /// Authorize the actor and return the pending row, or the current profile when the
    /// same upload is already active. Runs without a caller transaction so object-store
    /// inspection never holds PostgreSQL locks.
    fn prepare_finalize<'a>(
        &'a self,
        query: &'a PrepareAvatarFinalizeQuery,
    ) -> AvatarRepositoryFuture<'a, AvatarFinalizePreparation>;

    /// Re-lock the users row, release the current active row (queueing its object for
    /// deletion), activate this upload, update `users.avatar_url`, and return the profile.
    fn activate<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a ActivateAvatarCommand,
    ) -> AvatarRepositoryFuture<'a, UserProfile>;

    /// Resolve a publicly readable avatar; only `active` rows are visible.
    fn find_active<'a>(
        &'a self,
        avatar_id: Uuid,
    ) -> AvatarRepositoryFuture<'a, Option<ActiveAvatarRecord>>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateAvatarIntentCommand {
    pub id: Uuid,
    pub user_id: Uuid,
    pub object_key: String,
    pub byte_size: u64,
    pub expires_in: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AvatarIntentRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub object_key: String,
    pub byte_size: u64,
    pub expires_at: OffsetDateTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrepareAvatarFinalizeQuery {
    pub actor_id: Uuid,
    pub upload_id: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AvatarFinalizePreparation {
    Pending(AvatarIntentRecord),
    AlreadyActive(UserProfile),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivateAvatarCommand {
    pub actor_id: Uuid,
    pub upload_id: Uuid,
    pub public_url: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveAvatarRecord {
    pub object_key: String,
    pub byte_size: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AvatarRepositoryError {
    UploadNotFound,
    UploadNotPending,
    InvalidData,
    Unavailable,
}

impl fmt::Display for AvatarRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("avatar persistence operation failed")
    }
}

impl std::error::Error for AvatarRepositoryError {}
