//! Authoritative PostgreSQL media-upload persistence boundary.

use std::{fmt, future::Future, pin::Pin, time::Duration};

use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    domain::{
        media::{FinalizedObject, MediaKind, MediaScope},
        messaging::MessageAttachment,
    },
    ports::transactions::TransactionHandle,
};

pub type MediaRepositoryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, MediaRepositoryError>> + Send + 'a>>;

pub trait MediaRepository: Send + Sync {
    fn create_upload_intent<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a CreateUploadIntentCommand,
    ) -> MediaRepositoryFuture<'a, UploadIntentRecord>;

    /// Authorize the actor and return either the pending intent or its exact canonical retry.
    ///
    /// This read intentionally runs without a caller transaction so object-store inspection
    /// never holds PostgreSQL locks or a pooled connection.
    fn prepare_upload_finalize<'a>(
        &'a self,
        query: &'a PrepareUploadFinalizeQuery,
    ) -> MediaRepositoryFuture<'a, UploadFinalizePreparation>;

    /// Re-lock, re-authorize, and finalize using the caller-owned transaction.
    fn finalize_upload<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a FinalizeUploadCommand,
    ) -> MediaRepositoryFuture<'a, UploadFinalizeRecord>;

    /// Bind confirmed chat uploads in request order using the caller-owned transaction.
    ///
    /// Implementations must re-lock and re-authorize every capability. Object keys,
    /// filenames, and positions are deliberately absent from the command so they cannot
    /// become caller-controlled persistence input.
    fn bind_message_media<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _command: &'a BindMessageMediaCommand,
    ) -> MediaRepositoryFuture<'a, Vec<MessageAttachment>> {
        Box::pin(async { Err(MediaRepositoryError::Unavailable) })
    }

    /// Authorize one persisted attachment for the actor without exposing its object key.
    ///
    /// Implementations must derive group membership from the attachment's authoritative
    /// message/topic relation. A caller-supplied object key is deliberately absent.
    fn authorize_media_access<'a>(
        &'a self,
        _query: &'a AuthorizeMediaAccessQuery,
    ) -> MediaRepositoryFuture<'a, MediaAccessRecord> {
        Box::pin(async { Err(MediaRepositoryError::Unavailable) })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateUploadIntentCommand {
    pub id: Uuid,
    pub user_id: Uuid,
    pub scope: MediaScope,
    pub target_id: Uuid,
    pub object_key: String,
    pub kind: MediaKind,
    pub content_type: String,
    pub byte_size: u64,
    pub filename: Option<String>,
    pub expires_in: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadIntentRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub scope: MediaScope,
    pub target_id: Uuid,
    pub object_key: String,
    pub kind: MediaKind,
    pub content_type: String,
    pub byte_size: u64,
    pub filename: Option<String>,
    pub expires_at: OffsetDateTime,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrepareUploadFinalizeQuery {
    pub actor_id: Uuid,
    pub upload_id: Uuid,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UploadFinalizePreparation {
    Pending(UploadIntentRecord),
    Existing(UploadFinalizeRecord),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinalizeUploadCommand {
    Chat {
        actor_id: Uuid,
        upload_id: Uuid,
        finalized: FinalizedObject,
    },
    Topic {
        actor_id: Uuid,
        upload_id: Uuid,
        topic_media_id: Uuid,
        width: Option<u32>,
        height: Option<u32>,
        finalized: FinalizedObject,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UploadFinalizeRecord {
    Chat {
        upload: ConfirmedUploadRecord,
    },
    Topic {
        upload: ConfirmedUploadRecord,
        topic_media: TopicMediaBindingRecord,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfirmedUploadRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub scope: MediaScope,
    pub target_id: Uuid,
    pub object_key: String,
    pub kind: MediaKind,
    pub content_type: String,
    pub byte_size: u64,
    pub duration_seconds: Option<u64>,
    pub filename: Option<String>,
    pub confirmed_at: OffsetDateTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopicMediaBindingRecord {
    pub id: Uuid,
    pub topic_id: Uuid,
    pub media_upload_id: Uuid,
    pub object_key: String,
    pub content_type: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub byte_size: u64,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindMessageMediaCommand {
    pub actor_id: Uuid,
    pub chatroom_id: Uuid,
    pub message_id: Uuid,
    /// Provider-observed metadata in the exact request order.
    pub media: Vec<BindMessageMediaItem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindMessageMediaItem {
    pub upload_id: Uuid,
    pub finalized: FinalizedObject,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorizeMediaAccessQuery {
    pub actor_id: Uuid,
    pub media_id: Uuid,
}

/// DB-owned metadata for one authorized message or topic attachment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaAccessRecord {
    pub id: Uuid,
    pub media_upload_id: Uuid,
    pub object_key: String,
    pub content_type: String,
    pub byte_size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_seconds: Option<u64>,
    pub filename: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaRepositoryError {
    TargetNotAccessible,
    FinalizeConflict,
    InvalidData,
    Unavailable,
}

impl fmt::Display for MediaRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("media persistence operation failed")
    }
}

impl std::error::Error for MediaRepositoryError {}
