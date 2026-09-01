//! Atomic PostgreSQL binding for finalized chat uploads.

use std::collections::{HashMap, HashSet};

use sqlx::{PgConnection, Row, postgres::PgRow};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    domain::{
        media::{
            MAX_MEDIA_PER_MESSAGE, MediaKind, MediaScope, MessageMediaCandidate, mint_object_key,
            validate_message_composition, validate_upload,
        },
        messaging::MessageAttachment,
    },
    ports::media::{
        AuthoritativeMessageMediaCommand, BindMessageMediaCommand, BindMessageMediaItem,
        MediaRepositoryError,
    },
};

pub(super) async fn bind_authoritative_message_media(
    connection: &mut PgConnection,
    command: &AuthoritativeMessageMediaCommand,
) -> Result<Vec<MessageAttachment>, MediaRepositoryError> {
    let binding = BindMessageMediaCommand {
        actor_id: command.actor_id,
        chatroom_id: command.chatroom_id,
        message_id: command.message_id,
        media: command
            .upload_ids
            .iter()
            .copied()
            .map(|upload_id| BindMessageMediaItem {
                upload_id,
                // The first bind pass only uses IDs to acquire all authorized upload locks.
                // The derived value is replaced from the locked row immediately below.
                finalized: crate::domain::media::FinalizedObject {
                    kind: MediaKind::Image,
                    content_type: String::new(),
                    byte_size: 0,
                    duration_seconds: None,
                },
            })
            .collect(),
    };
    let _body = lock_authorized_message(connection, &binding).await?;
    validate_request_identity(&binding)?;
    let uploads = lock_uploads_in_request_order(connection, &binding).await?;
    let authoritative = BindMessageMediaCommand {
        media: uploads
            .iter()
            .map(|upload| {
                Ok(BindMessageMediaItem {
                    upload_id: upload.id,
                    finalized: upload.finalized_object()?,
                })
            })
            .collect::<Result<Vec<_>, MediaRepositoryError>>()?,
        ..binding
    };
    bind_message_media(connection, &authoritative).await
}

pub(super) async fn bind_message_media(
    connection: &mut PgConnection,
    command: &BindMessageMediaCommand,
) -> Result<Vec<MessageAttachment>, MediaRepositoryError> {
    let body = lock_authorized_message(connection, command).await?;
    validate_request_identity(command)?;

    if command.media.is_empty() {
        validate_message_composition(body.as_deref(), &[])
            .map_err(|_| MediaRepositoryError::InvalidData)?;
        return match lock_existing_attachments(connection, command.message_id).await? {
            attachments if attachments.is_empty() => Ok(Vec::new()),
            _ => Err(MediaRepositoryError::FinalizeConflict),
        };
    }

    let uploads = lock_uploads_in_request_order(connection, command).await?;
    let candidates = uploads
        .iter()
        .map(|upload| {
            Ok(MessageMediaCandidate {
                upload_id: upload.id,
                object_key: upload.object_key.clone(),
                kind: upload.validated_kind()?,
            })
        })
        .collect::<Result<Vec<_>, MediaRepositoryError>>()?;
    let composition = validate_message_composition(body.as_deref(), &candidates)
        .map_err(|_| MediaRepositoryError::InvalidData)?;

    let confirmed = uploads.iter().all(StoredUpload::is_confirmed);
    let bound = uploads.iter().all(StoredUpload::is_bound);
    if !confirmed && !bound {
        return Err(MediaRepositoryError::FinalizeConflict);
    }
    for (item, upload) in command.media.iter().zip(&uploads) {
        if item.finalized != upload.finalized_object()? {
            return Err(MediaRepositoryError::FinalizeConflict);
        }
    }

    match (confirmed, bound) {
        (true, false) => {
            if uploads.iter().any(|upload| !upload.is_live) {
                return Err(MediaRepositoryError::FinalizeConflict);
            }
            let existing = lock_existing_attachments(connection, command.message_id).await?;
            if !existing.is_empty() {
                return Err(MediaRepositoryError::FinalizeConflict);
            }
            bind_confirmed_uploads(connection, command, &uploads, &composition.media).await
        }
        (false, true) => {
            if uploads
                .iter()
                .any(|upload| upload.bound_message_id != Some(command.message_id))
            {
                return Err(MediaRepositoryError::FinalizeConflict);
            }
            exact_retry(connection, command.message_id, &uploads, &composition.media).await
        }
        _ => Err(MediaRepositoryError::FinalizeConflict),
    }
}

async fn lock_authorized_message(
    connection: &mut PgConnection,
    command: &BindMessageMediaCommand,
) -> Result<Option<String>, MediaRepositoryError> {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT stored_message.body \
         FROM messages AS stored_message \
         JOIN chatrooms AS chatroom \
           ON chatroom.id = stored_message.chatroom_id \
         JOIN groups AS live_group \
           ON live_group.id = chatroom.group_id \
          AND live_group.deleted_at IS NULL \
         JOIN memberships AS actor_membership \
           ON actor_membership.group_id = live_group.id \
          AND actor_membership.user_id = $3 \
         WHERE stored_message.id = $1 \
           AND stored_message.chatroom_id = $2 \
           AND stored_message.sender_id = $3 \
           AND stored_message.type = 'user' \
         FOR UPDATE OF stored_message \
         FOR SHARE OF chatroom, live_group, actor_membership",
    )
    .bind(command.message_id)
    .bind(command.chatroom_id)
    .bind(command.actor_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| super::database_error("message_media_authorize", error))?
    .ok_or(MediaRepositoryError::TargetNotAccessible)
}

fn validate_request_identity(
    command: &BindMessageMediaCommand,
) -> Result<(), MediaRepositoryError> {
    if command.media.len() > MAX_MEDIA_PER_MESSAGE {
        return Err(MediaRepositoryError::InvalidData);
    }
    let mut upload_ids = HashSet::with_capacity(command.media.len());
    if command
        .media
        .iter()
        .any(|item| !upload_ids.insert(item.upload_id))
    {
        return Err(MediaRepositoryError::InvalidData);
    }
    Ok(())
}

async fn lock_uploads_in_request_order(
    connection: &mut PgConnection,
    command: &BindMessageMediaCommand,
) -> Result<Vec<StoredUpload>, MediaRepositoryError> {
    let upload_ids = command
        .media
        .iter()
        .map(|item| item.upload_id)
        .collect::<Vec<_>>();
    let rows = sqlx::query(
        "SELECT upload.id, upload.user_id, upload.object_key, upload.scope, upload.target_id, \
                upload.content_type, upload.byte_size, upload.duration, upload.filename, \
                upload.status, upload.bound_message_id, upload.bound_topic_media_id, \
                upload.confirmed_at, upload.consumed_at, \
                upload.expires_at > clock_timestamp() AS is_live \
         FROM media_uploads AS upload \
         WHERE upload.id = ANY($1) \
           AND upload.user_id = $2 \
           AND upload.scope = 'chat' \
           AND upload.target_id = $3 \
         ORDER BY upload.id \
         FOR UPDATE OF upload",
    )
    .bind(&upload_ids[..])
    .bind(command.actor_id)
    .bind(command.chatroom_id)
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| super::database_error("message_media_lock_uploads", error))?;

    if rows.len() != upload_ids.len() {
        return Err(MediaRepositoryError::TargetNotAccessible);
    }

    let mut by_id = HashMap::with_capacity(rows.len());
    for row in &rows {
        let upload = StoredUpload::from_row(row)?;
        if by_id.insert(upload.id, upload).is_some() {
            return Err(MediaRepositoryError::InvalidData);
        }
    }

    command
        .media
        .iter()
        .map(|item| {
            by_id
                .remove(&item.upload_id)
                .ok_or(MediaRepositoryError::TargetNotAccessible)
        })
        .collect()
}

async fn bind_confirmed_uploads(
    connection: &mut PgConnection,
    command: &BindMessageMediaCommand,
    uploads: &[StoredUpload],
    ordered: &[crate::domain::media::OrderedMessageMedia],
) -> Result<Vec<MessageAttachment>, MediaRepositoryError> {
    let consumed_at = sqlx::query_scalar::<_, OffsetDateTime>("SELECT clock_timestamp()")
        .fetch_one(&mut *connection)
        .await
        .map_err(|error| super::database_error("message_media_timestamp", error))?;
    let mut attachments = Vec::with_capacity(uploads.len());

    for (upload, order) in uploads.iter().zip(ordered) {
        let updated = sqlx::query_scalar::<_, Uuid>(
            "UPDATE media_uploads AS upload \
             SET status = 'bound', bound_message_id = $2, consumed_at = $3 \
             WHERE upload.id = $1 \
               AND upload.user_id = $4 \
               AND upload.scope = 'chat' \
               AND upload.target_id = $5 \
               AND upload.status = 'confirmed' \
               AND upload.bound_message_id IS NULL \
               AND upload.bound_topic_media_id IS NULL \
               AND upload.consumed_at IS NULL \
               AND upload.expires_at > $3 \
             RETURNING upload.id",
        )
        .bind(upload.id)
        .bind(command.message_id)
        .bind(consumed_at)
        .bind(command.actor_id)
        .bind(command.chatroom_id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(|error| message_binding_database_error("message_media_consume", error))?;
        if updated != Some(upload.id) {
            return Err(MediaRepositoryError::FinalizeConflict);
        }

        let row = sqlx::query(
            "INSERT INTO message_media \
                 (id, message_id, media_upload_id, type, object_key, byte_size, duration, \
                  position, filename, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
             RETURNING id, media_upload_id, type, object_key, width, height, byte_size, \
                       duration, filename, position",
        )
        .bind(Uuid::new_v4())
        .bind(command.message_id)
        .bind(upload.id)
        .bind(&upload.content_type)
        .bind(&upload.object_key)
        .bind(upload.byte_size)
        .bind(upload.duration)
        .bind(i32::from(order.position))
        .bind(&upload.filename)
        .bind(consumed_at)
        .fetch_one(&mut *connection)
        .await
        .map_err(|error| message_binding_database_error("message_media_insert", error))?;
        attachments.push(StoredMessageMedia::from_row(&row)?.into_attachment()?);
    }

    Ok(attachments)
}

async fn exact_retry(
    connection: &mut PgConnection,
    message_id: Uuid,
    uploads: &[StoredUpload],
    ordered: &[crate::domain::media::OrderedMessageMedia],
) -> Result<Vec<MessageAttachment>, MediaRepositoryError> {
    let existing = lock_existing_attachments(connection, message_id).await?;
    if existing.len() != uploads.len() {
        return Err(MediaRepositoryError::FinalizeConflict);
    }

    let mut attachments = Vec::with_capacity(existing.len());
    for ((stored, upload), order) in existing.iter().zip(uploads).zip(ordered) {
        if stored.media_upload_id != upload.id
            || stored.object_key != upload.object_key
            || stored.content_type != upload.content_type
            || stored.byte_size != upload.byte_size
            || stored.duration != upload.duration
            || stored.filename != upload.filename
            || stored.width.is_some()
            || stored.height.is_some()
            || stored.position != i32::from(order.position)
        {
            return Err(MediaRepositoryError::FinalizeConflict);
        }
        attachments.push(stored.clone().into_attachment()?);
    }
    Ok(attachments)
}

async fn lock_existing_attachments(
    connection: &mut PgConnection,
    message_id: Uuid,
) -> Result<Vec<StoredMessageMedia>, MediaRepositoryError> {
    sqlx::query(
        "SELECT media.id, media.media_upload_id, media.type, media.object_key, \
                media.width, media.height, media.byte_size, media.duration, \
                media.filename, media.position \
         FROM message_media AS media \
         WHERE media.message_id = $1 \
         ORDER BY media.position \
         FOR SHARE OF media",
    )
    .bind(message_id)
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| super::database_error("message_media_existing", error))?
    .iter()
    .map(StoredMessageMedia::from_row)
    .collect()
}

#[derive(Clone, Debug)]
struct StoredUpload {
    id: Uuid,
    object_key: String,
    content_type: String,
    byte_size: i64,
    duration: Option<i32>,
    filename: Option<String>,
    status: String,
    bound_message_id: Option<Uuid>,
    bound_topic_media_id: Option<Uuid>,
    confirmed_at: Option<OffsetDateTime>,
    consumed_at: Option<OffsetDateTime>,
    is_live: bool,
}

impl StoredUpload {
    fn from_row(row: &PgRow) -> Result<Self, MediaRepositoryError> {
        let id = required(row, "id")?;
        let user_id: Uuid = required(row, "user_id")?;
        let scope: String = required(row, "scope")?;
        let target_id: Uuid = required(row, "target_id")?;
        let upload = Self {
            id,
            object_key: required(row, "object_key")?,
            content_type: required(row, "content_type")?,
            byte_size: required(row, "byte_size")?,
            duration: required(row, "duration")?,
            filename: required(row, "filename")?,
            status: required(row, "status")?,
            bound_message_id: required(row, "bound_message_id")?,
            bound_topic_media_id: required(row, "bound_topic_media_id")?,
            confirmed_at: required(row, "confirmed_at")?,
            consumed_at: required(row, "consumed_at")?,
            is_live: required(row, "is_live")?,
        };
        if scope != "chat"
            || upload.object_key != mint_object_key(MediaScope::Chat, target_id, id)
            || user_id.is_nil()
        {
            return Err(MediaRepositoryError::InvalidData);
        }
        Ok(upload)
    }

    fn validated_kind(&self) -> Result<MediaKind, MediaRepositoryError> {
        validate_upload(
            MediaScope::Chat,
            &self.content_type,
            checked_u64(self.byte_size)?,
            self.filename.as_deref(),
        )
        .map(|validated| validated.kind)
        .map_err(|_| MediaRepositoryError::InvalidData)
    }

    fn finalized_object(
        &self,
    ) -> Result<crate::domain::media::FinalizedObject, MediaRepositoryError> {
        let kind = self.validated_kind()?;
        let duration_seconds = match (kind, self.duration) {
            (MediaKind::Audio, Some(duration)) if duration > 0 => {
                Some(u64::try_from(duration).map_err(|_| MediaRepositoryError::InvalidData)?)
            }
            (MediaKind::Audio, _) => return Err(MediaRepositoryError::InvalidData),
            (_, None) => None,
            (_, Some(_)) => return Err(MediaRepositoryError::InvalidData),
        };
        Ok(crate::domain::media::FinalizedObject {
            kind,
            content_type: self.content_type.clone(),
            byte_size: checked_u64(self.byte_size)?,
            duration_seconds,
        })
    }

    fn is_confirmed(&self) -> bool {
        self.status == "confirmed"
            && self.confirmed_at.is_some()
            && self.consumed_at.is_none()
            && self.bound_message_id.is_none()
            && self.bound_topic_media_id.is_none()
    }

    fn is_bound(&self) -> bool {
        self.status == "bound"
            && self.confirmed_at.is_some()
            && self.consumed_at.is_some()
            && self.bound_message_id.is_some()
            && self.bound_topic_media_id.is_none()
    }
}

#[derive(Clone, Debug)]
struct StoredMessageMedia {
    id: Uuid,
    media_upload_id: Uuid,
    content_type: String,
    object_key: String,
    width: Option<i32>,
    height: Option<i32>,
    byte_size: i64,
    duration: Option<i32>,
    filename: Option<String>,
    position: i32,
}

impl StoredMessageMedia {
    fn from_row(row: &PgRow) -> Result<Self, MediaRepositoryError> {
        Ok(Self {
            id: required(row, "id")?,
            media_upload_id: required(row, "media_upload_id")?,
            content_type: required(row, "type")?,
            object_key: required(row, "object_key")?,
            width: required(row, "width")?,
            height: required(row, "height")?,
            byte_size: required(row, "byte_size")?,
            duration: required(row, "duration")?,
            filename: required(row, "filename")?,
            position: required(row, "position")?,
        })
    }

    fn into_attachment(self) -> Result<MessageAttachment, MediaRepositoryError> {
        Ok(MessageAttachment {
            id: self.id,
            media_upload_id: self.media_upload_id,
            content_type: self.content_type,
            byte_size: checked_u64(self.byte_size)?,
            width: checked_optional_u32(self.width)?,
            height: checked_optional_u32(self.height)?,
            duration: checked_optional_u64(self.duration)?,
            filename: self.filename,
            position: u8::try_from(self.position).map_err(|_| MediaRepositoryError::InvalidData)?,
        })
    }
}

fn checked_u64(value: i64) -> Result<u64, MediaRepositoryError> {
    u64::try_from(value).map_err(|_| MediaRepositoryError::InvalidData)
}

fn checked_optional_u32(value: Option<i32>) -> Result<Option<u32>, MediaRepositoryError> {
    value
        .map(|value| {
            if value <= 0 {
                return Err(MediaRepositoryError::InvalidData);
            }
            u32::try_from(value).map_err(|_| MediaRepositoryError::InvalidData)
        })
        .transpose()
}

fn checked_optional_u64(value: Option<i32>) -> Result<Option<u64>, MediaRepositoryError> {
    value
        .map(|value| {
            if value <= 0 {
                return Err(MediaRepositoryError::InvalidData);
            }
            u64::try_from(value).map_err(|_| MediaRepositoryError::InvalidData)
        })
        .transpose()
}

fn required<'row, Value>(row: &'row PgRow, column: &str) -> Result<Value, MediaRepositoryError>
where
    Value: sqlx::Decode<'row, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    row.try_get(column)
        .map_err(|_| MediaRepositoryError::InvalidData)
}

fn message_binding_database_error(
    operation: &'static str,
    error: sqlx::Error,
) -> MediaRepositoryError {
    if let sqlx::Error::Database(database) = &error {
        match database.constraint() {
            Some(
                "uq_message_media_upload"
                | "uq_message_media_object_key"
                | "uq_message_media_message_position"
                | "fk_message_media_upload"
                | "fk_message_media_bound_upload",
            ) => return MediaRepositoryError::FinalizeConflict,
            Some(
                "media_uploads_consumer_shape_check"
                | "message_media_type_check"
                | "message_media_object_key_check"
                | "message_media_width_check"
                | "message_media_height_check"
                | "message_media_byte_size_check"
                | "message_media_duration_check"
                | "message_media_position_check"
                | "message_media_filename_check",
            ) => return MediaRepositoryError::InvalidData,
            _ => {}
        }
    }
    tracing::warn!(
        dependency = "postgres",
        failure_kind = "message_media_binding",
        operation,
        "PostgreSQL message-media binding failed"
    );
    MediaRepositoryError::Unavailable
}
