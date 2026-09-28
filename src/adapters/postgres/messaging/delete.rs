use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    domain::messaging::{
        MessageDeletedData, MessageDeletedEvent, MessageDeletedType, RealtimeServerEvent,
    },
    ports::{
        messaging::{DeleteMessageCommand, MessagingRepositoryError},
        transactions::TransactionHandle,
    },
};

type MessageTargetRow = (Option<Uuid>, Option<OffsetDateTime>, String);
type MediaObjectRow = (Uuid, String, Option<Uuid>, Option<String>);

pub(super) async fn delete_message(
    handle: &mut dyn TransactionHandle,
    command: &DeleteMessageCommand,
) -> Result<(), MessagingRepositoryError> {
    let connection = crate::adapters::postgres::transactions::connection(handle)
        .map_err(|_| database_error("transaction_handle"))?;
    let group_id = lock_chatroom_access(connection, command).await?;
    let Some(deleted_at) = lock_authorized_message(connection, command).await? else {
        return Ok(());
    };
    let media = lock_message_media(connection, command.message_id).await?;
    enqueue_object_deletions(connection, &media).await?;
    mark_media_uploads_deleted(connection, &media, deleted_at).await?;
    delete_message_media(connection, command.message_id).await?;
    scrub_message_row(connection, command.message_id, deleted_at).await?;
    let created_events = scrub_message_created_events(connection, command).await?;
    scrub_message_created_outbox(connection, &created_events).await?;
    stop_message_push_occurrences(connection, command.message_id, deleted_at).await?;
    hide_message_notifications(connection, command.chatroom_id, &created_events, deleted_at)
        .await?;
    append_message_deleted_event(connection, command, group_id, deleted_at).await
}

async fn lock_chatroom_access(
    connection: &mut PgConnection,
    command: &DeleteMessageCommand,
) -> Result<Uuid, MessagingRepositoryError> {
    let group_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT live_group.id \
         FROM chatrooms AS chatroom \
         JOIN groups AS live_group \
           ON live_group.id = chatroom.group_id \
          AND live_group.deleted_at IS NULL \
         LEFT JOIN topics AS topic \
           ON topic.id = chatroom.topic_id \
         WHERE chatroom.id = $1 \
           AND chatroom.deleted_at IS NULL \
           AND (chatroom.topic_id IS NULL OR topic.deleted_at IS NULL) \
         FOR UPDATE OF live_group, chatroom",
    )
    .bind(command.chatroom_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| database_error("delete_chatroom_lock"))?
    .ok_or(MessagingRepositoryError::MessageNotFound)?;

    let membership = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM memberships \
         WHERE group_id = $1 AND user_id = $2 AND deleted_at IS NULL \
         FOR SHARE",
    )
    .bind(group_id)
    .bind(command.actor_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| database_error("delete_membership_lock"))?;
    if membership.is_none() {
        return Err(MessagingRepositoryError::MembershipRequired);
    }
    Ok(group_id)
}

async fn lock_authorized_message(
    connection: &mut PgConnection,
    command: &DeleteMessageCommand,
) -> Result<Option<OffsetDateTime>, MessagingRepositoryError> {
    let row = sqlx::query_as::<_, MessageTargetRow>(
        "SELECT sender_id, deleted_at, type \
         FROM messages \
         WHERE id = $1 AND chatroom_id = $2 \
         FOR UPDATE",
    )
    .bind(command.message_id)
    .bind(command.chatroom_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| database_error("delete_message_lock"))?
    .ok_or(MessagingRepositoryError::MessageNotFound)?;

    if row.1.is_some() {
        return if row.0 == Some(command.actor_id) {
            Ok(None)
        } else {
            Err(MessagingRepositoryError::MessageNotFound)
        };
    }
    if row.0 != Some(command.actor_id) || row.2 != "user" {
        return Err(MessagingRepositoryError::MessageAuthorRequired);
    }
    let deleted_at = sqlx::query_scalar::<_, OffsetDateTime>("SELECT clock_timestamp()")
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| database_error("delete_timestamp"))?;
    Ok(Some(deleted_at))
}

async fn lock_message_media(
    connection: &mut PgConnection,
    message_id: Uuid,
) -> Result<Vec<MediaObjectRow>, MessagingRepositoryError> {
    sqlx::query_as::<_, MediaObjectRow>(
        "SELECT upload.id, media.object_key, poster.id, poster.object_key \
         FROM message_media AS media \
         JOIN media_uploads AS upload ON upload.id = media.media_upload_id \
         LEFT JOIN media_uploads AS poster ON poster.id = upload.poster_upload_id \
         WHERE media.message_id = $1 \
         ORDER BY media.position \
         FOR UPDATE OF media, upload",
    )
    .bind(message_id)
    .fetch_all(connection)
    .await
    .map_err(|_| database_error("delete_media_lock"))
}

async fn enqueue_object_deletions(
    connection: &mut PgConnection,
    media: &[MediaObjectRow],
) -> Result<(), MessagingRepositoryError> {
    let mut object_keys = Vec::new();
    for row in media {
        object_keys.push(row.1.clone());
        if let Some(object_key) = &row.3 {
            object_keys.push(object_key.clone());
        }
    }
    object_keys.sort();
    object_keys.dedup();
    for object_key in object_keys {
        sqlx::query(
            "INSERT INTO account_object_deletion_intents (id, object_key) \
             VALUES ($1, $2) \
             ON CONFLICT (object_key) DO NOTHING",
        )
        .bind(Uuid::new_v4())
        .bind(object_key)
        .execute(&mut *connection)
        .await
        .map_err(|_| database_error("delete_object_intent_insert"))?;
    }
    Ok(())
}

async fn mark_media_uploads_deleted(
    connection: &mut PgConnection,
    media: &[MediaObjectRow],
    deleted_at: OffsetDateTime,
) -> Result<(), MessagingRepositoryError> {
    let mut upload_ids = Vec::new();
    for row in media {
        upload_ids.push(row.0);
        if let Some(poster_id) = row.2 {
            upload_ids.push(poster_id);
        }
    }
    upload_ids.sort();
    upload_ids.dedup();
    if upload_ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE media_uploads \
         SET deleted_at = COALESCE(deleted_at, $2) \
         WHERE id = ANY($1)",
    )
    .bind(upload_ids)
    .bind(deleted_at)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(|_| database_error("delete_media_uploads"))
}

async fn delete_message_media(
    connection: &mut PgConnection,
    message_id: Uuid,
) -> Result<(), MessagingRepositoryError> {
    sqlx::query("DELETE FROM message_media WHERE message_id = $1")
        .bind(message_id)
        .execute(connection)
        .await
        .map(|_| ())
        .map_err(|_| database_error("delete_message_media"))
}

async fn scrub_message_row(
    connection: &mut PgConnection,
    message_id: Uuid,
    deleted_at: OffsetDateTime,
) -> Result<(), MessagingRepositoryError> {
    sqlx::query(
        "UPDATE messages \
         SET deleted_at = $2, body = NULL \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(message_id)
    .bind(deleted_at)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(|_| database_error("delete_message_scrub"))
}

async fn scrub_message_created_events(
    connection: &mut PgConnection,
    command: &DeleteMessageCommand,
) -> Result<Vec<(Uuid, i64)>, MessagingRepositoryError> {
    sqlx::query_as::<_, (Uuid, i64)>(
        "UPDATE conversation_events \
         SET payload = jsonb_set( \
                 jsonb_set(payload, '{body}', 'null'::jsonb, true), \
                 '{media}', '[]'::jsonb, true \
             ) \
         WHERE conversation_id = $1 \
           AND event_type = 'message.created' \
           AND event_version = 1 \
           AND payload ->> 'id' = $2::uuid::text \
         RETURNING id, cursor",
    )
    .bind(command.chatroom_id)
    .bind(command.message_id)
    .fetch_all(connection)
    .await
    .map_err(|_| database_error("delete_message_event_scrub"))
}

async fn scrub_message_created_outbox(
    connection: &mut PgConnection,
    events: &[(Uuid, i64)],
) -> Result<(), MessagingRepositoryError> {
    let event_ids = events.iter().map(|event| event.0).collect::<Vec<_>>();
    if event_ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE outbox_events \
         SET payload = jsonb_set( \
                 jsonb_set(payload, '{data,body}', 'null'::jsonb, true), \
                 '{data,media}', '[]'::jsonb, true \
             ) \
         WHERE event_type = 'message.created' \
           AND conversation_event_id = ANY($1)",
    )
    .bind(event_ids)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(|_| database_error("delete_message_outbox_scrub"))
}

async fn stop_message_push_occurrences(
    connection: &mut PgConnection,
    message_id: Uuid,
    deleted_at: OffsetDateTime,
) -> Result<(), MessagingRepositoryError> {
    sqlx::query(
        "UPDATE push_delivery_intents \
         SET status = 'failed', \
             claim_owner = NULL, \
             lease_expires_at = NULL, \
             failed_at = COALESCE(failed_at, $2), \
             last_error_code = 'source_deleted', \
             source_message_id = NULL, \
             payload = jsonb_set(payload, '{message_id}', 'null'::jsonb, true) \
         WHERE source_message_id = $1 \
           AND status IN ('pending', 'claimed', 'retryable')",
    )
    .bind(message_id)
    .bind(deleted_at)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(|_| database_error("delete_message_push_stop"))
}

async fn hide_message_notifications(
    connection: &mut PgConnection,
    chatroom_id: Uuid,
    events: &[(Uuid, i64)],
    deleted_at: OffsetDateTime,
) -> Result<(), MessagingRepositoryError> {
    let cursors = events.iter().map(|event| event.1).collect::<Vec<_>>();
    if cursors.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE notifications \
         SET deleted_at = COALESCE(deleted_at, $3), \
             read_at = COALESCE(read_at, $3), \
             payload = '{}'::jsonb \
         WHERE conversation_id = $1 \
           AND source_cursor = ANY($2) \
           AND deleted_at IS NULL",
    )
    .bind(chatroom_id)
    .bind(cursors)
    .bind(deleted_at)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(|_| database_error("delete_message_notifications"))
}

async fn append_message_deleted_event(
    connection: &mut PgConnection,
    command: &DeleteMessageCommand,
    group_id: Uuid,
    deleted_at: OffsetDateTime,
) -> Result<(), MessagingRepositoryError> {
    let event_id = Uuid::new_v4();
    let data = MessageDeletedData {
        message_id: command.message_id,
        chatroom_id: command.chatroom_id,
        group_id,
        deleted_at,
        deleted_by: command.actor_id,
        reason: "author_deleted".to_owned(),
    };
    let payload = serde_json::to_value(&data).map_err(|_| database_error("delete_payload"))?;
    let (cursor, occurred_at) = sqlx::query_as::<_, (i64, OffsetDateTime)>(
        "INSERT INTO conversation_events \
             (id, conversation_id, event_type, event_version, payload) \
         VALUES ($1, $2, 'message.deleted', 1, $3) \
         RETURNING cursor, occurred_at",
    )
    .bind(event_id)
    .bind(command.chatroom_id)
    .bind(payload)
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| database_error("delete_event_insert"))?;
    let event = RealtimeServerEvent::MessageDeleted(MessageDeletedEvent {
        version: 1,
        event_type: MessageDeletedType::MessageDeleted,
        event_id,
        conversation_id: command.chatroom_id,
        cursor: cursor.to_string(),
        occurred_at,
        data,
    });
    let outbox_payload =
        serde_json::to_value(event).map_err(|_| database_error("delete_outbox_payload"))?;
    sqlx::query(
        "INSERT INTO outbox_events \
             (id, intent_type, event_type, event_version, aggregate_type, aggregate_id, \
              conversation_event_id, payload) \
         VALUES ($1, 'conversation', 'message.deleted', 1, 'conversation', $2, $3, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(command.chatroom_id)
    .bind(event_id)
    .bind(outbox_payload)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(|_| database_error("delete_outbox_insert"))
}

fn database_error(operation: &'static str) -> MessagingRepositoryError {
    tracing::warn!(
        dependency = "postgres",
        failure_kind = "message_delete",
        operation,
        "PostgreSQL message delete failed"
    );
    MessagingRepositoryError::DatabaseUnavailable
}
