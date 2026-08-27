//! Authorized media attachment lookup without transaction or object-store I/O.

use sqlx::{PgPool, Row, postgres::PgRow};

use crate::ports::media::{AuthorizeMediaAccessQuery, MediaAccessRecord, MediaRepositoryError};

const AUTHORIZED_MEDIA_SQL: &str = "WITH authorized_media AS ( \
         SELECT attachment.id, upload.id AS media_upload_id, \
                attachment.object_key, attachment.type AS content_type, \
                attachment.byte_size, attachment.width, attachment.height, \
                upload.duration, upload.filename \
         FROM message_media AS attachment \
         JOIN messages AS stored_message \
           ON stored_message.id = attachment.message_id \
         JOIN media_uploads AS upload \
           ON upload.id = attachment.media_upload_id \
          AND upload.scope = 'chat' \
          AND upload.status = 'bound' \
          AND upload.bound_message_id = attachment.message_id \
          AND upload.target_id = stored_message.chatroom_id \
          AND upload.object_key = attachment.object_key \
          AND upload.content_type = attachment.type \
          AND upload.byte_size = attachment.byte_size \
         JOIN chatrooms AS chatroom \
           ON chatroom.id = stored_message.chatroom_id \
         JOIN groups AS live_group \
           ON live_group.id = chatroom.group_id \
          AND live_group.deleted_at IS NULL \
         JOIN memberships AS actor_membership \
           ON actor_membership.group_id = live_group.id \
          AND actor_membership.user_id = $2 \
         WHERE attachment.id = $1 \
         UNION ALL \
         SELECT attachment.id, upload.id AS media_upload_id, \
                attachment.object_key, attachment.type AS content_type, \
                attachment.byte_size, attachment.width, attachment.height, \
                upload.duration, upload.filename \
         FROM topic_media AS attachment \
         JOIN media_uploads AS upload \
           ON upload.id = attachment.media_upload_id \
          AND upload.scope = 'topic' \
          AND upload.status = 'bound' \
          AND upload.bound_topic_media_id = attachment.id \
          AND upload.target_id = attachment.topic_id \
          AND upload.object_key = attachment.object_key \
          AND upload.content_type = attachment.type \
          AND upload.byte_size = attachment.byte_size \
         JOIN topics AS topic \
           ON topic.id = attachment.topic_id \
         JOIN groups AS live_group \
           ON live_group.id = topic.group_id \
          AND live_group.deleted_at IS NULL \
         JOIN memberships AS actor_membership \
           ON actor_membership.group_id = live_group.id \
          AND actor_membership.user_id = $2 \
         WHERE attachment.id = $1 \
     ) \
     SELECT id, media_upload_id, object_key, content_type, byte_size, width, height, \
            duration, filename \
     FROM authorized_media \
     WHERE (SELECT count(*) FROM authorized_media) = 1";

pub(super) async fn authorize_media_access(
    pool: &PgPool,
    query: &AuthorizeMediaAccessQuery,
) -> Result<MediaAccessRecord, MediaRepositoryError> {
    let row = sqlx::query(AUTHORIZED_MEDIA_SQL)
        .bind(query.media_id)
        .bind(query.actor_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| super::database_error("media_access_authorize", error))?
        .ok_or(MediaRepositoryError::TargetNotAccessible)?;

    media_access_record(&row)
}

fn media_access_record(row: &PgRow) -> Result<MediaAccessRecord, MediaRepositoryError> {
    let object_key = required::<String>(row, "object_key")?;
    let content_type = required::<String>(row, "content_type")?;
    if object_key.is_empty() || content_type.is_empty() {
        return Err(MediaRepositoryError::InvalidData);
    }

    Ok(MediaAccessRecord {
        id: required(row, "id")?,
        media_upload_id: required(row, "media_upload_id")?,
        object_key,
        content_type,
        byte_size: positive_u64(required(row, "byte_size")?)?,
        width: positive_optional_u32(required(row, "width")?)?,
        height: positive_optional_u32(required(row, "height")?)?,
        duration_seconds: positive_optional_u64(required(row, "duration")?)?,
        filename: required(row, "filename")?,
    })
}

fn positive_u64(value: i64) -> Result<u64, MediaRepositoryError> {
    if value <= 0 {
        return Err(MediaRepositoryError::InvalidData);
    }
    u64::try_from(value).map_err(|_| MediaRepositoryError::InvalidData)
}

fn positive_optional_u32(value: Option<i32>) -> Result<Option<u32>, MediaRepositoryError> {
    value
        .map(|value| {
            if value <= 0 {
                return Err(MediaRepositoryError::InvalidData);
            }
            u32::try_from(value).map_err(|_| MediaRepositoryError::InvalidData)
        })
        .transpose()
}

fn positive_optional_u64(value: Option<i32>) -> Result<Option<u64>, MediaRepositoryError> {
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
