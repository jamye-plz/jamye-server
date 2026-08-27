//! Authoritative media finalization queries and row mapping.

use sqlx::{PgConnection, PgPool, Row, postgres::PgRow};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    domain::media::{FinalizedObject, MediaKind, MediaScope, validate_upload},
    ports::media::{
        ConfirmedUploadRecord, FinalizeUploadCommand, MediaRepositoryError,
        PrepareUploadFinalizeQuery, TopicMediaBindingRecord, UploadFinalizePreparation,
        UploadFinalizeRecord, UploadIntentRecord,
    },
};

const AUTHORIZED_UPLOAD_SQL: &str = "SELECT upload.id, upload.user_id, upload.object_key, upload.scope, upload.target_id, \
            upload.content_type, upload.byte_size, upload.duration, upload.filename, \
            upload.status, upload.bound_message_id, upload.bound_topic_media_id, \
            upload.confirmed_at, upload.consumed_at, upload.expires_at, upload.created_at, \
            upload.expires_at > clock_timestamp() AS is_live \
     FROM media_uploads AS upload \
     WHERE upload.id = $1 \
       AND upload.user_id = $2 \
       AND ( \
           ( \
               upload.scope = 'chat' \
               AND EXISTS ( \
                   SELECT 1 \
                   FROM chatrooms AS chatroom \
                   JOIN groups AS live_group \
                     ON live_group.id = chatroom.group_id \
                    AND live_group.deleted_at IS NULL \
                   JOIN memberships AS actor_membership \
                     ON actor_membership.group_id = chatroom.group_id \
                    AND actor_membership.user_id = $2 \
                   WHERE chatroom.id = upload.target_id \
                   FOR SHARE OF chatroom, live_group, actor_membership \
               ) \
           ) \
           OR ( \
               upload.scope = 'topic' \
               AND EXISTS ( \
                   SELECT 1 \
                   FROM topics AS topic \
                   JOIN groups AS live_group \
                     ON live_group.id = topic.group_id \
                    AND live_group.deleted_at IS NULL \
                   JOIN memberships AS actor_membership \
                     ON actor_membership.group_id = topic.group_id \
                    AND actor_membership.user_id = $2 \
                   WHERE topic.id = upload.target_id \
                     AND (topic.author_id = $2 OR live_group.owner_id = $2) \
                   FOR SHARE OF topic, live_group, actor_membership \
               ) \
           ) \
       ) \
     FOR UPDATE OF upload";

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
        return upload
            .intent_record()
            .map(UploadFinalizePreparation::Pending);
    }

    match upload.scope {
        MediaScope::Chat if upload.status == "confirmed" => {
            if !upload.confirmed_chat_shape() {
                return Err(MediaRepositoryError::InvalidData);
            }
            let record = UploadFinalizeRecord::Chat {
                upload: upload.confirmed_record()?,
            };
            Ok(UploadFinalizePreparation::Existing(record))
        }
        MediaScope::Topic if upload.status == "bound" => {
            if !upload.bound_topic_shape() {
                return Err(MediaRepositoryError::InvalidData);
            }
            let topic_media_id = upload
                .bound_topic_media_id
                .ok_or(MediaRepositoryError::InvalidData)?;
            let row = sqlx::query(
                "SELECT id, topic_id, media_upload_id, object_key, type, width, height, \
                        byte_size, created_at \
                 FROM topic_media \
                 WHERE id = $1 AND media_upload_id = $2 AND topic_id = $3",
            )
            .bind(topic_media_id)
            .bind(upload.id)
            .bind(upload.target_id)
            .fetch_optional(pool)
            .await
            .map_err(|error| super::database_error("upload_finalize_retry_topic", error))?
            .ok_or(MediaRepositoryError::InvalidData)?;
            let topic_media = StoredTopicMedia::from_row(&row)?.into_record()?;

            if topic_media.object_key != upload.object_key
                || topic_media.content_type != upload.content_type
                || topic_media.byte_size != checked_u64(upload.byte_size)?
            {
                return Err(MediaRepositoryError::InvalidData);
            }
            if topic_media.width != query.width || topic_media.height != query.height {
                return Err(MediaRepositoryError::FinalizeConflict);
            }

            let record = UploadFinalizeRecord::Topic {
                upload: upload.confirmed_record()?,
                topic_media,
            };
            Ok(UploadFinalizePreparation::Existing(record))
        }
        _ => Err(MediaRepositoryError::FinalizeConflict),
    }
}

pub(super) async fn finalize_upload(
    connection: &mut PgConnection,
    command: &FinalizeUploadCommand,
) -> Result<UploadFinalizeRecord, MediaRepositoryError> {
    let (actor_id, upload_id) = match command {
        FinalizeUploadCommand::Chat {
            actor_id,
            upload_id,
            ..
        }
        | FinalizeUploadCommand::Topic {
            actor_id,
            upload_id,
            ..
        } => (*actor_id, *upload_id),
    };
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

    match command {
        FinalizeUploadCommand::Chat { finalized, .. } => {
            let duration = validate_finalized(&upload, MediaScope::Chat, finalized)?;
            let confirmed_at = sqlx::query_scalar::<_, OffsetDateTime>(
                "WITH stamped AS (SELECT clock_timestamp() AS at) \
                 UPDATE media_uploads AS upload \
                 SET status = 'confirmed', duration = $2, confirmed_at = stamped.at \
                 FROM stamped \
                 WHERE upload.id = $1 AND upload.status = 'pending' \
                 RETURNING upload.confirmed_at",
            )
            .bind(upload.id)
            .bind(duration)
            .fetch_optional(&mut *connection)
            .await
            .map_err(|error| finalize_database_error("upload_finalize_chat", error))?
            .ok_or(MediaRepositoryError::FinalizeConflict)?;

            Ok(UploadFinalizeRecord::Chat {
                upload: upload.finalized_record(finalized, confirmed_at),
            })
        }
        FinalizeUploadCommand::Topic {
            topic_media_id,
            width,
            height,
            finalized,
            ..
        } => {
            let duration = validate_finalized(&upload, MediaScope::Topic, finalized)?;
            let recorded_width = *width;
            let recorded_height = *height;
            let database_width = checked_dimension(recorded_width)?;
            let database_height = checked_dimension(recorded_height)?;
            let confirmed_at = sqlx::query_scalar::<_, OffsetDateTime>(
                "WITH stamped AS (SELECT clock_timestamp() AS at) \
                 UPDATE media_uploads AS upload \
                 SET status = 'bound', duration = $2, bound_topic_media_id = $3, \
                     confirmed_at = stamped.at, consumed_at = stamped.at \
                 FROM stamped \
                 WHERE upload.id = $1 AND upload.status = 'pending' \
                 RETURNING upload.confirmed_at",
            )
            .bind(upload.id)
            .bind(duration)
            .bind(*topic_media_id)
            .fetch_optional(&mut *connection)
            .await
            .map_err(|error| finalize_database_error("upload_finalize_topic", error))?
            .ok_or(MediaRepositoryError::FinalizeConflict)?;

            sqlx::query(
                "INSERT INTO topic_media \
                     (id, topic_id, media_upload_id, type, object_key, width, height, byte_size, \
                      created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(topic_media_id)
            .bind(upload.target_id)
            .bind(upload.id)
            .bind(&finalized.content_type)
            .bind(&upload.object_key)
            .bind(database_width)
            .bind(database_height)
            .bind(upload.byte_size)
            .bind(confirmed_at)
            .execute(&mut *connection)
            .await
            .map_err(|error| finalize_database_error("upload_finalize_topic_media", error))?;

            Ok(UploadFinalizeRecord::Topic {
                upload: upload.finalized_record(finalized, confirmed_at),
                topic_media: TopicMediaBindingRecord {
                    id: *topic_media_id,
                    topic_id: upload.target_id,
                    media_upload_id: upload.id,
                    object_key: upload.object_key.clone(),
                    content_type: finalized.content_type.clone(),
                    width: recorded_width,
                    height: recorded_height,
                    byte_size: finalized.byte_size,
                    created_at: confirmed_at,
                },
            })
        }
    }
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
    bound_topic_media_id: Option<Uuid>,
    confirmed_at: Option<OffsetDateTime>,
    consumed_at: Option<OffsetDateTime>,
    expires_at: OffsetDateTime,
    created_at: OffsetDateTime,
    is_live: bool,
}

impl StoredUpload {
    fn from_row(row: &PgRow) -> Result<Self, MediaRepositoryError> {
        let stored_scope: String = required(row, "scope")?;
        let scope = match stored_scope.as_str() {
            "chat" => MediaScope::Chat,
            "topic" => MediaScope::Topic,
            _ => return Err(MediaRepositoryError::InvalidData),
        };
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
            bound_topic_media_id: required(row, "bound_topic_media_id")?,
            confirmed_at: required(row, "confirmed_at")?,
            consumed_at: required(row, "consumed_at")?,
            expires_at: required(row, "expires_at")?,
            created_at: required(row, "created_at")?,
            is_live: required(row, "is_live")?,
        })
    }

    fn pending_shape(&self) -> bool {
        self.confirmed_at.is_none()
            && self.consumed_at.is_none()
            && self.bound_message_id.is_none()
            && self.bound_topic_media_id.is_none()
            && self.duration.is_none()
    }

    fn confirmed_chat_shape(&self) -> bool {
        self.scope == MediaScope::Chat
            && self.confirmed_at.is_some()
            && self.consumed_at.is_none()
            && self.bound_message_id.is_none()
            && self.bound_topic_media_id.is_none()
    }

    fn bound_topic_shape(&self) -> bool {
        self.scope == MediaScope::Topic
            && self.confirmed_at.is_some()
            && self.consumed_at.is_some()
            && self.bound_message_id.is_none()
            && self.bound_topic_media_id.is_some()
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
        })
    }

    fn finalized_record(
        &self,
        finalized: &FinalizedObject,
        confirmed_at: OffsetDateTime,
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
        }
    }
}

struct StoredTopicMedia {
    id: Uuid,
    topic_id: Uuid,
    media_upload_id: Uuid,
    object_key: String,
    content_type: String,
    width: Option<i32>,
    height: Option<i32>,
    byte_size: Option<i64>,
    created_at: OffsetDateTime,
}

impl StoredTopicMedia {
    fn from_row(row: &PgRow) -> Result<Self, MediaRepositoryError> {
        Ok(Self {
            id: required(row, "id")?,
            topic_id: required(row, "topic_id")?,
            media_upload_id: required(row, "media_upload_id")?,
            object_key: required(row, "object_key")?,
            content_type: required(row, "type")?,
            width: required(row, "width")?,
            height: required(row, "height")?,
            byte_size: required(row, "byte_size")?,
            created_at: required(row, "created_at")?,
        })
    }

    fn into_record(self) -> Result<TopicMediaBindingRecord, MediaRepositoryError> {
        Ok(TopicMediaBindingRecord {
            id: self.id,
            topic_id: self.topic_id,
            media_upload_id: self.media_upload_id,
            object_key: self.object_key,
            content_type: self.content_type,
            width: checked_stored_dimension(self.width)?,
            height: checked_stored_dimension(self.height)?,
            byte_size: checked_u64(self.byte_size.ok_or(MediaRepositoryError::InvalidData)?)?,
            created_at: self.created_at,
        })
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

fn checked_dimension(value: Option<u32>) -> Result<Option<i32>, MediaRepositoryError> {
    value
        .map(|value| {
            if value == 0 {
                return Err(MediaRepositoryError::FinalizeConflict);
            }
            i32::try_from(value).map_err(|_| MediaRepositoryError::FinalizeConflict)
        })
        .transpose()
}

fn checked_stored_dimension(value: Option<i32>) -> Result<Option<u32>, MediaRepositoryError> {
    value
        .map(|value| {
            if value <= 0 {
                return Err(MediaRepositoryError::InvalidData);
            }
            u32::try_from(value).map_err(|_| MediaRepositoryError::InvalidData)
        })
        .transpose()
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
            Some(
                "uq_media_uploads_bound_topic_media"
                | "uq_topic_media_upload"
                | "uq_topic_media_topic_object"
                | "fk_media_uploads_bound_topic_media"
                | "fk_topic_media_upload"
                | "fk_topic_media_bound_upload",
            ) => return MediaRepositoryError::FinalizeConflict,
            Some(
                "media_uploads_duration_check"
                | "media_uploads_consumer_shape_check"
                | "topic_media_type_check"
                | "topic_media_object_key_check"
                | "topic_media_width_check"
                | "topic_media_height_check"
                | "topic_media_byte_size_check",
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
