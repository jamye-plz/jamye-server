//! Authoritative media finalization queries and row mapping.

use sqlx::{PgConnection, PgPool, Row, postgres::PgRow};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    domain::media::{FinalizedObject, MediaKind, MediaScope, validate_upload},
    ports::media::{
        ConfirmedUploadRecord, FinalizeUploadCommand, MediaRepositoryError, PosterCandidateRecord,
        PrepareUploadFinalizeQuery, UploadFinalizePreparation, UploadFinalizeRecord,
        UploadIntentRecord,
    },
};

fn parse_media_scope(value: &str) -> Result<MediaScope, MediaRepositoryError> {
    match value {
        "chat" => Ok(MediaScope::Chat),
        _ => Err(MediaRepositoryError::InvalidData),
    }
}

const AUTHORIZED_UPLOAD_SQL: &str = "SELECT upload.id, upload.user_id, upload.object_key, upload.scope, upload.target_id, \
            upload.content_type, upload.byte_size, upload.duration, upload.filename, \
            upload.status, upload.bound_message_id, upload.confirmed_at, upload.consumed_at, \
            upload.expires_at, upload.created_at, upload.poster_upload_id, \
            upload.expires_at > clock_timestamp() AS is_live \
     FROM media_uploads AS upload \
     WHERE upload.id = $1 \
       AND upload.user_id = $2 \
       AND upload.scope = 'chat' \
       AND upload.deleted_at IS NULL \
       AND EXISTS ( \
           SELECT 1 \
           FROM chatrooms AS chatroom \
           JOIN groups AS live_group \
             ON live_group.id = chatroom.group_id \
            AND live_group.deleted_at IS NULL \
           JOIN memberships AS actor_membership \
             ON actor_membership.group_id = chatroom.group_id \
            AND actor_membership.user_id = $2 \
            AND actor_membership.deleted_at IS NULL \
           LEFT JOIN topics topic ON topic.id = chatroom.topic_id \
           WHERE chatroom.id = upload.target_id \
             AND chatroom.deleted_at IS NULL \
             AND (chatroom.topic_id IS NULL OR topic.deleted_at IS NULL) \
           FOR SHARE OF chatroom, live_group, actor_membership \
       ) \
     FOR UPDATE OF upload";

/// Load the poster candidate's own row without restricting by owner: ownership,
/// scope, and target matching against the video are authorized by the domain
/// poster policy so a mismatch surfaces as a validation error rather than a
/// silent not-found.
// Concurrency for poster linkage is enforced by the partial unique index
// `uq_media_uploads_poster_upload` at finalize commit time, not by a row lock here.
const POSTER_CANDIDATE_SQL: &str = "SELECT poster.id, poster.user_id, poster.scope, poster.target_id, \
            poster.content_type, poster.byte_size, poster.status, \
            poster.poster_upload_id AS own_poster_upload_id, \
            EXISTS ( \
                SELECT 1 FROM media_uploads AS linked \
                WHERE linked.poster_upload_id = poster.id \
                  AND linked.deleted_at IS NULL \
            ) AS already_linked \
     FROM media_uploads AS poster \
     WHERE poster.id = $1 AND poster.deleted_at IS NULL";

pub(super) async fn prepare_upload_finalize(
    pool: &PgPool,
    query: &PrepareUploadFinalizeQuery,
) -> Result<UploadFinalizePreparation, MediaRepositoryError> {
    let row = sqlx::query(AUTHORIZED_UPLOAD_SQL)
        .bind(query.upload_id)
        .bind(query.actor_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| super::database_error("upload_finalize_prepare", error))?
        .ok_or(MediaRepositoryError::TargetNotAccessible)?;
    let upload = StoredUpload::from_row(&row)?;

    if upload.status == "pending" {
        if !upload.pending_shape() {
            return Err(MediaRepositoryError::InvalidData);
        }
        if !upload.is_live {
            return Err(MediaRepositoryError::FinalizeConflict);
        }
        let poster = match query.poster_upload_id {
            Some(poster_upload_id) => load_poster_candidate(pool, poster_upload_id).await?,
            None => None,
        };
        return Ok(UploadFinalizePreparation::Pending {
            upload: upload.intent_record()?,
            poster,
        });
    }

    if upload.status == "confirmed" {
        if !upload.confirmed_chat_shape() {
            return Err(MediaRepositoryError::InvalidData);
        }
        return Ok(UploadFinalizePreparation::Existing(
            UploadFinalizeRecord::Chat {
                upload: upload.confirmed_record()?,
            },
        ));
    }

    Err(MediaRepositoryError::FinalizeConflict)
}

async fn load_poster_candidate(
    pool: &PgPool,
    poster_upload_id: Uuid,
) -> Result<Option<PosterCandidateRecord>, MediaRepositoryError> {
    let Some(row) = sqlx::query(POSTER_CANDIDATE_SQL)
        .bind(poster_upload_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| super::database_error("upload_finalize_prepare_poster", error))?
    else {
        return Ok(None);
    };
    let stored_scope: String = required(&row, "scope")?;
    let scope = parse_media_scope(&stored_scope)?;
    let content_type: String = required(&row, "content_type")?;
    let byte_size = checked_u64(required(&row, "byte_size")?)?;
    let kind = validate_upload(scope, &content_type, byte_size, None)
        .map(|validated| validated.kind)
        .map_err(|_| MediaRepositoryError::InvalidData)?;
    let status: String = required(&row, "status")?;
    let own_poster_upload_id: Option<Uuid> = required(&row, "own_poster_upload_id")?;
    Ok(Some(PosterCandidateRecord {
        id: required(&row, "id")?,
        user_id: required(&row, "user_id")?,
        scope,
        target_id: required(&row, "target_id")?,
        kind,
        content_type,
        byte_size,
        status_confirmed: status == "confirmed",
        already_linked: required(&row, "already_linked")?,
        has_own_poster: own_poster_upload_id.is_some(),
    }))
}

pub(super) async fn finalize_upload(
    connection: &mut PgConnection,
    command: &FinalizeUploadCommand,
) -> Result<UploadFinalizeRecord, MediaRepositoryError> {
    let FinalizeUploadCommand::Chat {
        actor_id,
        upload_id,
        finalized,
        poster_upload_id,
    } = command;
    let row = sqlx::query(AUTHORIZED_UPLOAD_SQL)
        .bind(upload_id)
        .bind(actor_id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(|error| finalize_database_error("upload_finalize_lock", error))?
        .ok_or(MediaRepositoryError::TargetNotAccessible)?;
    let upload = StoredUpload::from_row(&row)?;

    if upload.status != "pending" {
        return Err(MediaRepositoryError::FinalizeConflict);
    }
    if !upload.pending_shape() {
        return Err(MediaRepositoryError::InvalidData);
    }
    if !upload.is_live {
        return Err(MediaRepositoryError::FinalizeConflict);
    }

    let duration = validate_finalized(&upload, MediaScope::Chat, finalized)?;
    let confirmed_at = sqlx::query_scalar::<_, OffsetDateTime>(
        "WITH stamped AS (SELECT clock_timestamp() AS at) \
         UPDATE media_uploads AS upload \
         SET status = 'confirmed', duration = $2, confirmed_at = stamped.at, \
             poster_upload_id = $3 \
         FROM stamped \
         WHERE upload.id = $1 AND upload.status = 'pending' \
           AND upload.deleted_at IS NULL \
         RETURNING upload.confirmed_at",
    )
    .bind(upload.id)
    .bind(duration)
    .bind(poster_upload_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| finalize_database_error("upload_finalize_chat", error))?
    .ok_or(MediaRepositoryError::FinalizeConflict)?;

    Ok(UploadFinalizeRecord::Chat {
        upload: upload.finalized_record(finalized, confirmed_at, *poster_upload_id),
    })
}

#[derive(Clone, Debug)]
struct StoredUpload {
    id: Uuid,
    user_id: Uuid,
    object_key: String,
    scope: MediaScope,
    target_id: Uuid,
    content_type: String,
    byte_size: i64,
    duration: Option<i32>,
    filename: Option<String>,
    status: String,
    bound_message_id: Option<Uuid>,
    confirmed_at: Option<OffsetDateTime>,
    consumed_at: Option<OffsetDateTime>,
    expires_at: OffsetDateTime,
    created_at: OffsetDateTime,
    poster_upload_id: Option<Uuid>,
    is_live: bool,
}

impl StoredUpload {
    fn from_row(row: &PgRow) -> Result<Self, MediaRepositoryError> {
        let stored_scope: String = required(row, "scope")?;
        let scope = parse_media_scope(&stored_scope)?;
        Ok(Self {
            id: required(row, "id")?,
            user_id: required(row, "user_id")?,
            object_key: required(row, "object_key")?,
            scope,
            target_id: required(row, "target_id")?,
            content_type: required(row, "content_type")?,
            byte_size: required(row, "byte_size")?,
            duration: required(row, "duration")?,
            filename: required(row, "filename")?,
            status: required(row, "status")?,
            bound_message_id: required(row, "bound_message_id")?,
            confirmed_at: required(row, "confirmed_at")?,
            consumed_at: required(row, "consumed_at")?,
            expires_at: required(row, "expires_at")?,
            created_at: required(row, "created_at")?,
            poster_upload_id: required(row, "poster_upload_id")?,
            is_live: required(row, "is_live")?,
        })
    }

    fn pending_shape(&self) -> bool {
        self.confirmed_at.is_none()
            && self.consumed_at.is_none()
            && self.bound_message_id.is_none()
            && self.duration.is_none()
    }

    fn confirmed_chat_shape(&self) -> bool {
        self.scope == MediaScope::Chat
            && self.confirmed_at.is_some()
            && self.consumed_at.is_none()
            && self.bound_message_id.is_none()
    }

    fn validated_kind(&self) -> Result<MediaKind, MediaRepositoryError> {
        let byte_size = checked_u64(self.byte_size)?;
        validate_upload(
            self.scope,
            &self.content_type,
            byte_size,
            self.filename.as_deref(),
        )
        .map(|validated| validated.kind)
        .map_err(|_| MediaRepositoryError::InvalidData)
    }

    fn intent_record(&self) -> Result<UploadIntentRecord, MediaRepositoryError> {
        Ok(UploadIntentRecord {
            id: self.id,
            user_id: self.user_id,
            scope: self.scope,
            target_id: self.target_id,
            object_key: self.object_key.clone(),
            kind: self.validated_kind()?,
            content_type: self.content_type.clone(),
            byte_size: checked_u64(self.byte_size)?,
            filename: self.filename.clone(),
            expires_at: self.expires_at,
            created_at: self.created_at,
        })
    }

    fn confirmed_record(&self) -> Result<ConfirmedUploadRecord, MediaRepositoryError> {
        let kind = self.validated_kind()?;
        let duration_seconds = checked_stored_duration(kind, self.duration)?;
        Ok(ConfirmedUploadRecord {
            id: self.id,
            user_id: self.user_id,
            scope: self.scope,
            target_id: self.target_id,
            object_key: self.object_key.clone(),
            kind,
            content_type: self.content_type.clone(),
            byte_size: checked_u64(self.byte_size)?,
            duration_seconds,
            filename: self.filename.clone(),
            confirmed_at: self.confirmed_at.ok_or(MediaRepositoryError::InvalidData)?,
            poster_upload_id: self.poster_upload_id,
        })
    }

    fn finalized_record(
        &self,
        finalized: &FinalizedObject,
        confirmed_at: OffsetDateTime,
        poster_upload_id: Option<Uuid>,
    ) -> ConfirmedUploadRecord {
        ConfirmedUploadRecord {
            id: self.id,
            user_id: self.user_id,
            scope: self.scope,
            target_id: self.target_id,
            object_key: self.object_key.clone(),
            kind: finalized.kind,
            content_type: finalized.content_type.clone(),
            byte_size: finalized.byte_size,
            duration_seconds: finalized.duration_seconds,
            filename: self.filename.clone(),
            confirmed_at,
            poster_upload_id,
        }
    }
}

fn validate_finalized(
    upload: &StoredUpload,
    expected_scope: MediaScope,
    finalized: &FinalizedObject,
) -> Result<Option<i32>, MediaRepositoryError> {
    let kind = upload.validated_kind()?;
    if upload.scope != expected_scope
        || finalized.kind != kind
        || finalized.content_type != upload.content_type
        || finalized.byte_size != checked_u64(upload.byte_size)?
    {
        return Err(MediaRepositoryError::FinalizeConflict);
    }

    match (kind, finalized.duration_seconds) {
        (MediaKind::Audio, Some(duration)) if duration > 0 => i32::try_from(duration)
            .map(Some)
            .map_err(|_| MediaRepositoryError::FinalizeConflict),
        (MediaKind::Audio, _) => Err(MediaRepositoryError::FinalizeConflict),
        (_, None) => Ok(None),
        (_, Some(_)) => Err(MediaRepositoryError::FinalizeConflict),
    }
}

fn checked_stored_duration(
    kind: MediaKind,
    duration: Option<i32>,
) -> Result<Option<u64>, MediaRepositoryError> {
    match (kind, duration) {
        (MediaKind::Audio, Some(value)) if value > 0 => Ok(Some(
            u64::try_from(value).map_err(|_| MediaRepositoryError::InvalidData)?,
        )),
        (MediaKind::Audio, _) => Err(MediaRepositoryError::InvalidData),
        (_, None) => Ok(None),
        (_, Some(_)) => Err(MediaRepositoryError::InvalidData),
    }
}

fn checked_u64(value: i64) -> Result<u64, MediaRepositoryError> {
    u64::try_from(value).map_err(|_| MediaRepositoryError::InvalidData)
}

fn required<'row, Value>(row: &'row PgRow, column: &str) -> Result<Value, MediaRepositoryError>
where
    Value: sqlx::Decode<'row, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    row.try_get(column)
        .map_err(|_| MediaRepositoryError::InvalidData)
}

fn finalize_database_error(operation: &'static str, error: sqlx::Error) -> MediaRepositoryError {
    if let sqlx::Error::Database(database) = &error {
        match database.constraint() {
            Some("uq_media_uploads_poster_upload" | "fk_media_uploads_poster_upload") => {
                return MediaRepositoryError::FinalizeConflict;
            }
            Some(
                "media_uploads_duration_check"
                | "media_uploads_consumer_shape_check"
                | "media_uploads_poster_self_reference_check",
            ) => return MediaRepositoryError::InvalidData,
            _ => {}
        }
    }
    tracing::warn!(
        dependency = "postgres",
        failure_kind = "media_finalize",
        operation,
        "PostgreSQL media finalize operation failed"
    );
    MediaRepositoryError::Unavailable
}
