use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    domain::messaging::{
        MessageDeletedData, MessageDeletedEvent, MessageDeletedType, RealtimeServerEvent,
    },
    ports::{
        messaging::{
            DeleteMessageCommand, MessagingRepositoryError, ModeratedMessage,
            ModeratorDeleteMessageCommand, ModeratorDeleteOutcome,
        },
        transactions::TransactionHandle,
    },
};

type MessageTargetRow = (Option<Uuid>, Option<OffsetDateTime>, String);
type MediaObjectRow = (Uuid, String, Option<Uuid>, Option<String>);

const AUTHOR_DELETE_REASON: &str = "author_deleted";
/// Reason carried by the delete event of an operator deletion.
const MODERATOR_DELETE_REASON: &str = "moderator_deleted";

/// Everything the shared removal steps need; the author and the operator path differ only in
/// how they authorize and in the actor and reason recorded on the delete event.
struct Removal {
    chatroom_id: Uuid,
    message_id: Uuid,
    group_id: Uuid,
    deleted_by: Uuid,
    reason: &'static str,
}

pub(super) async fn delete_message(
    handle: &mut dyn TransactionHandle,
    command: &DeleteMessageCommand,
) -> Result<(), MessagingRepositoryError> {
    let connection = crate::adapters::postgres::transactions::connection(handle)
        .map_err(|_| database_error("transaction_handle"))?;
    let group_id = lock_live_chatroom(connection, command.chatroom_id).await?;
    require_membership(connection, group_id, command.actor_id).await?;
    let Some(deleted_at) = lock_authorized_message(connection, command).await? else {
        return Ok(());
    };
    remove_message(
        connection,
        &Removal {
            chatroom_id: command.chatroom_id,
            message_id: command.message_id,
            group_id,
            deleted_by: command.actor_id,
            reason: AUTHOR_DELETE_REASON,
        },
        deleted_at,
    )
    .await
}

/// Operator path: resolve the chatroom from the message id, lock the live chatroom and group in
/// the same order as the author path, then run the shared removal. An operator has no account
/// actor, so the event records the nil UUID as `deleted_by`.
pub(super) async fn moderator_delete_message(
    handle: &mut dyn TransactionHandle,
    command: &ModeratorDeleteMessageCommand,
) -> Result<ModeratorDeleteOutcome, MessagingRepositoryError> {
    let connection = crate::adapters::postgres::transactions::connection(handle)
        .map_err(|_| database_error("transaction_handle"))?;
    let chatroom_id =
        sqlx::query_scalar::<_, Uuid>("SELECT chatroom_id FROM messages WHERE id = $1")
            .bind(command.message_id)
            .fetch_optional(&mut *connection)
            .await
            .map_err(|_| database_error("moderator_delete_lookup"))?
            .ok_or(MessagingRepositoryError::MessageNotFound)?;
    let group_id = lock_live_chatroom(connection, chatroom_id).await?;
    let moderated = ModeratedMessage {
        chatroom_id,
        group_id,
    };
    let row = fetch_locked_message(connection, chatroom_id, command.message_id).await?;
    if row.1.is_some() {
        return Ok(ModeratorDeleteOutcome::AlreadyDeleted(moderated));
    }
    if row.2 != "user" {
        return Ok(ModeratorDeleteOutcome::NotUserMessage);
    }
    let deleted_at = server_timestamp(connection).await?;
    remove_message(
        connection,
        &Removal {
            chatroom_id,
            message_id: command.message_id,
            group_id,
            deleted_by: Uuid::nil(),
            reason: MODERATOR_DELETE_REASON,
        },
        deleted_at,
    )
    .await?;
    Ok(ModeratorDeleteOutcome::Deleted(moderated))
}

async fn remove_message(
    connection: &mut PgConnection,
    removal: &Removal,
    deleted_at: OffsetDateTime,
) -> Result<(), MessagingRepositoryError> {
    let media = lock_message_media(connection, removal.message_id).await?;
    enqueue_object_deletions(connection, &media).await?;
    mark_media_uploads_deleted(connection, &media, deleted_at).await?;
    delete_message_media(connection, removal.message_id).await?;
    scrub_message_row(connection, removal.message_id, deleted_at).await?;
    let created_events = scrub_message_created_events(connection, removal).await?;
    scrub_message_created_outbox(connection, &created_events).await?;
    stop_message_push_occurrences(connection, removal.message_id, deleted_at).await?;
    hide_message_notifications(connection, removal.chatroom_id, &created_events, deleted_at)
        .await?;
    append_message_deleted_event(connection, removal, deleted_at).await
}

/// Locks the live group and chatroom (a deleted topic hides its chatroom) and returns the group.
async fn lock_live_chatroom(
    connection: &mut PgConnection,
    chatroom_id: Uuid,
) -> Result<Uuid, MessagingRepositoryError> {
    sqlx::query_scalar::<_, Uuid>(
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
    .bind(chatroom_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| database_error("delete_chatroom_lock"))?
    .ok_or(MessagingRepositoryError::MessageNotFound)
}

async fn require_membership(
    connection: &mut PgConnection,
    group_id: Uuid,
    actor_id: Uuid,
) -> Result<(), MessagingRepositoryError> {
    let membership = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM memberships \
         WHERE group_id = $1 AND user_id = $2 AND deleted_at IS NULL \
         FOR SHARE",
    )
    .bind(group_id)
    .bind(actor_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| database_error("delete_membership_lock"))?;
    if membership.is_none() {
        return Err(MessagingRepositoryError::MembershipRequired);
    }
    Ok(())
}

async fn fetch_locked_message(
    connection: &mut PgConnection,
    chatroom_id: Uuid,
    message_id: Uuid,
) -> Result<MessageTargetRow, MessagingRepositoryError> {
    sqlx::query_as::<_, MessageTargetRow>(
        "SELECT sender_id, deleted_at, type \
         FROM messages \
         WHERE id = $1 AND chatroom_id = $2 \
         FOR UPDATE",
    )
    .bind(message_id)
    .bind(chatroom_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| database_error("delete_message_lock"))?
    .ok_or(MessagingRepositoryError::MessageNotFound)
}

async fn server_timestamp(
    connection: &mut PgConnection,
) -> Result<OffsetDateTime, MessagingRepositoryError> {
    sqlx::query_scalar::<_, OffsetDateTime>("SELECT clock_timestamp()")
        .fetch_one(connection)
        .await
        .map_err(|_| database_error("delete_timestamp"))
}

async fn lock_authorized_message(
    connection: &mut PgConnection,
    command: &DeleteMessageCommand,
) -> Result<Option<OffsetDateTime>, MessagingRepositoryError> {
    let row = fetch_locked_message(connection, command.chatroom_id, command.message_id).await?;

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
    server_timestamp(connection).await.map(Some)
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
    removal: &Removal,
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
    .bind(removal.chatroom_id)
    .bind(removal.message_id)
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
         SET deleted_at = COALESCE(deleted_at, $2), \
             status = 'failed', \
             claim_owner = NULL, \
             lease_expires_at = NULL, \
             failed_at = COALESCE(failed_at, $2), \
             last_error_code = 'source_deleted', \
             source_message_id = NULL, \
             payload = jsonb_set(payload, '{message_id}', 'null'::jsonb, true) \
         WHERE source_message_id = $1 \
           AND deleted_at IS NULL \
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
    removal: &Removal,
    deleted_at: OffsetDateTime,
) -> Result<(), MessagingRepositoryError> {
    let event_id = Uuid::new_v4();
    let data = MessageDeletedData {
        message_id: removal.message_id,
        chatroom_id: removal.chatroom_id,
        group_id: removal.group_id,
        deleted_at,
        deleted_by: removal.deleted_by,
        reason: removal.reason.to_owned(),
    };
    let payload = serde_json::to_value(&data).map_err(|_| database_error("delete_payload"))?;
    let (cursor, occurred_at) = sqlx::query_as::<_, (i64, OffsetDateTime)>(
        "INSERT INTO conversation_events \
             (id, conversation_id, event_type, event_version, payload) \
         VALUES ($1, $2, 'message.deleted', 1, $3) \
         RETURNING cursor, occurred_at",
    )
    .bind(event_id)
    .bind(removal.chatroom_id)
    .bind(payload)
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| database_error("delete_event_insert"))?;
    let event = RealtimeServerEvent::MessageDeleted(MessageDeletedEvent {
        version: 1,
        event_type: MessageDeletedType::MessageDeleted,
        event_id,
        conversation_id: removal.chatroom_id,
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
    .bind(removal.chatroom_id)
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
