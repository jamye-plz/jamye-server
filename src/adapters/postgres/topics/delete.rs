use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    domain::messaging::{
        MessageDeletedData, MessageDeletedEvent, MessageDeletedType, RealtimeServerEvent,
        TopicDeletedData, TopicDeletedEvent, TopicDeletedType,
    },
    ports::topics::{DeleteTopicCommand, TopicsRepositoryError},
};

use super::database_error;

type TopicTargetRow = (Uuid, Option<OffsetDateTime>, Uuid, Uuid);
type MediaObjectRow = (Uuid, String, Option<Uuid>, Option<String>);
type ScrubbedMessageEvent = (Uuid, Uuid, i64);

pub(super) async fn delete_topic(
    connection: &mut PgConnection,
    command: &DeleteTopicCommand,
) -> Result<(), TopicsRepositoryError> {
    lock_group_and_membership(connection, command.group_id, command.actor_id).await?;
    let Some((topic_author_id, topic_chatroom_id, main_chatroom_id, deleted_at)) =
        lock_authorized_topic(connection, command).await?
    else {
        return Ok(());
    };

    let announcement_ids =
        lock_announcement_messages(connection, command, topic_author_id, main_chatroom_id).await?;
    let mut message_ids = lock_topic_messages(connection, topic_chatroom_id).await?;
    message_ids.extend(announcement_ids.iter().copied());
    message_ids.sort();
    message_ids.dedup();

    let media = lock_message_media(connection, &message_ids).await?;
    enqueue_object_deletions(connection, &media).await?;
    mark_media_uploads_deleted(connection, &media, deleted_at).await?;
    delete_message_media(connection, &message_ids).await?;
    scrub_messages(connection, &message_ids, deleted_at).await?;
    let message_events = scrub_message_created_events(connection, &message_ids).await?;
    scrub_message_created_outbox(connection, &message_events).await?;
    let topic_events = scrub_topic_created_events(connection, command, topic_chatroom_id).await?;
    scrub_topic_created_outbox(connection, &topic_events).await?;
    stop_push_occurrences(connection, &message_ids, &topic_events, deleted_at).await?;
    hide_notifications(
        connection,
        command.topic_id,
        topic_chatroom_id,
        main_chatroom_id,
        &message_events,
        deleted_at,
    )
    .await?;
    delete_topic_side_tables(connection, command.topic_id, topic_chatroom_id).await?;
    scrub_topic_row_and_chatroom(connection, command.topic_id, topic_chatroom_id, deleted_at)
        .await?;
    for message_id in &announcement_ids {
        append_message_deleted_event(
            connection,
            *message_id,
            main_chatroom_id,
            command.group_id,
            command.actor_id,
            deleted_at,
        )
        .await?;
    }
    append_topic_deleted_event(
        connection,
        command,
        topic_chatroom_id,
        main_chatroom_id,
        announcement_ids.first().copied(),
        deleted_at,
    )
    .await
}

async fn lock_group_and_membership(
    connection: &mut PgConnection,
    group_id: Uuid,
    actor_id: Uuid,
) -> Result<(), TopicsRepositoryError> {
    let live = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM groups \
         WHERE id = $1 AND deleted_at IS NULL \
         FOR UPDATE",
    )
    .bind(group_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| database_error("topic_delete_group_lock", error))?;
    if live.is_none() {
        return Err(TopicsRepositoryError::GroupNotFound);
    }
    let membership = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM memberships \
         WHERE group_id = $1 AND user_id = $2 AND deleted_at IS NULL \
         FOR SHARE",
    )
    .bind(group_id)
    .bind(actor_id)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("topic_delete_membership_lock", error))?;
    if membership.is_none() {
        return Err(TopicsRepositoryError::MembershipRequired);
    }
    Ok(())
}

async fn lock_authorized_topic(
    connection: &mut PgConnection,
    command: &DeleteTopicCommand,
) -> Result<Option<(Uuid, Uuid, Uuid, OffsetDateTime)>, TopicsRepositoryError> {
    let row = sqlx::query_as::<_, TopicTargetRow>(
        "SELECT topic.author_id, topic.deleted_at, topic_chatroom.id, main_chatroom.id \
         FROM topics AS topic \
         JOIN chatrooms AS topic_chatroom \
           ON topic_chatroom.group_id = topic.group_id \
          AND topic_chatroom.topic_id = topic.id \
          AND topic_chatroom.type = 'topic' \
         JOIN chatrooms AS main_chatroom \
           ON main_chatroom.group_id = topic.group_id \
          AND main_chatroom.type = 'main' \
          AND main_chatroom.topic_id IS NULL \
         WHERE topic.id = $1 AND topic.group_id = $2 \
         FOR UPDATE OF topic, topic_chatroom, main_chatroom",
    )
    .bind(command.topic_id)
    .bind(command.group_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| database_error("topic_delete_lock", error))?
    .ok_or(TopicsRepositoryError::TopicNotFound)?;

    if row.1.is_some() {
        return if row.0 == command.actor_id {
            Ok(None)
        } else {
            Err(TopicsRepositoryError::TopicNotFound)
        };
    }
    if row.0 != command.actor_id {
        return Err(TopicsRepositoryError::AuthorRequired);
    }
    let deleted_at = sqlx::query_scalar::<_, OffsetDateTime>("SELECT clock_timestamp()")
        .fetch_one(connection)
        .await
        .map_err(|error| database_error("topic_delete_timestamp", error))?;
    Ok(Some((row.0, row.2, row.3, deleted_at)))
}

async fn lock_announcement_messages(
    connection: &mut PgConnection,
    command: &DeleteTopicCommand,
    author_id: Uuid,
    main_chatroom_id: Uuid,
) -> Result<Vec<Uuid>, TopicsRepositoryError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM messages \
         WHERE chatroom_id = $1 \
           AND sender_id = $2 \
           AND type = 'user' \
           AND deleted_at IS NULL \
           AND announcement_for_topic_id = $3 \
         ORDER BY created_at, id \
         FOR UPDATE",
    )
    .bind(main_chatroom_id)
    .bind(author_id)
    .bind(command.topic_id)
    .fetch_all(connection)
    .await
    .map_err(|error| database_error("topic_delete_announcement_lock", error))
}

async fn lock_topic_messages(
    connection: &mut PgConnection,
    topic_chatroom_id: Uuid,
) -> Result<Vec<Uuid>, TopicsRepositoryError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM messages \
         WHERE chatroom_id = $1 AND deleted_at IS NULL \
         ORDER BY created_at, id \
         FOR UPDATE",
    )
    .bind(topic_chatroom_id)
    .fetch_all(connection)
    .await
    .map_err(|error| database_error("topic_delete_message_lock", error))
}

async fn lock_message_media(
    connection: &mut PgConnection,
    message_ids: &[Uuid],
) -> Result<Vec<MediaObjectRow>, TopicsRepositoryError> {
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as::<_, MediaObjectRow>(
        "SELECT upload.id, media.object_key, poster.id, poster.object_key \
         FROM message_media AS media \
         JOIN media_uploads AS upload ON upload.id = media.media_upload_id \
         LEFT JOIN media_uploads AS poster ON poster.id = upload.poster_upload_id \
         WHERE media.message_id = ANY($1) \
         ORDER BY media.message_id, media.position \
         FOR UPDATE OF media, upload",
    )
    .bind(message_ids.to_vec())
    .fetch_all(connection)
    .await
    .map_err(|error| database_error("topic_delete_media_lock", error))
}

async fn enqueue_object_deletions(
    connection: &mut PgConnection,
    media: &[MediaObjectRow],
) -> Result<(), TopicsRepositoryError> {
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
        .map_err(|error| database_error("topic_delete_object_intent", error))?;
    }
    Ok(())
}

async fn mark_media_uploads_deleted(
    connection: &mut PgConnection,
    media: &[MediaObjectRow],
    deleted_at: OffsetDateTime,
) -> Result<(), TopicsRepositoryError> {
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
    .map_err(|error| database_error("topic_delete_media_uploads", error))
}

async fn delete_message_media(
    connection: &mut PgConnection,
    message_ids: &[Uuid],
) -> Result<(), TopicsRepositoryError> {
    if message_ids.is_empty() {
        return Ok(());
    }
    sqlx::query("DELETE FROM message_media WHERE message_id = ANY($1)")
        .bind(message_ids.to_vec())
        .execute(connection)
        .await
        .map(|_| ())
        .map_err(|error| database_error("topic_delete_message_media", error))
}

async fn scrub_messages(
    connection: &mut PgConnection,
    message_ids: &[Uuid],
    deleted_at: OffsetDateTime,
) -> Result<(), TopicsRepositoryError> {
    if message_ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE messages \
         SET deleted_at = COALESCE(deleted_at, $2), body = NULL \
         WHERE id = ANY($1)",
    )
    .bind(message_ids.to_vec())
    .bind(deleted_at)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(|error| database_error("topic_delete_messages", error))
}

async fn scrub_message_created_events(
    connection: &mut PgConnection,
    message_ids: &[Uuid],
) -> Result<Vec<ScrubbedMessageEvent>, TopicsRepositoryError> {
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }
    let message_ids = message_ids.iter().map(Uuid::to_string).collect::<Vec<_>>();
    sqlx::query_as::<_, ScrubbedMessageEvent>(
        "UPDATE conversation_events \
         SET payload = jsonb_set( \
                 jsonb_set(payload, '{body}', 'null'::jsonb, true), \
                 '{media}', '[]'::jsonb, true \
             ) \
         WHERE event_type = 'message.created' \
           AND event_version = 1 \
           AND payload ->> 'id' = ANY($1) \
         RETURNING id, conversation_id, cursor",
    )
    .bind(message_ids)
    .fetch_all(connection)
    .await
    .map_err(|error| database_error("topic_delete_message_event_scrub", error))
}

async fn scrub_message_created_outbox(
    connection: &mut PgConnection,
    events: &[ScrubbedMessageEvent],
) -> Result<(), TopicsRepositoryError> {
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
    .map_err(|error| database_error("topic_delete_message_outbox_scrub", error))
}

async fn scrub_topic_created_events(
    connection: &mut PgConnection,
    command: &DeleteTopicCommand,
    topic_chatroom_id: Uuid,
) -> Result<Vec<Uuid>, TopicsRepositoryError> {
    sqlx::query_scalar::<_, Uuid>(
        "UPDATE conversation_events \
         SET payload = jsonb_set(payload, '{title}', to_jsonb(''::text), true) \
         WHERE conversation_id = $1 \
           AND event_type = 'topic.created' \
           AND event_version = 1 \
           AND payload ->> 'topic_id' = $2::uuid::text \
         RETURNING id",
    )
    .bind(topic_chatroom_id)
    .bind(command.topic_id)
    .fetch_all(connection)
    .await
    .map_err(|error| database_error("topic_delete_topic_event_scrub", error))
}

async fn scrub_topic_created_outbox(
    connection: &mut PgConnection,
    event_ids: &[Uuid],
) -> Result<(), TopicsRepositoryError> {
    if event_ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE outbox_events \
         SET payload = jsonb_set(payload, '{data,title}', to_jsonb(''::text), true) \
         WHERE event_type = 'topic.created' \
           AND conversation_event_id = ANY($1)",
    )
    .bind(event_ids)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(|error| database_error("topic_delete_topic_outbox_scrub", error))
}

async fn stop_push_occurrences(
    connection: &mut PgConnection,
    message_ids: &[Uuid],
    topic_event_ids: &[Uuid],
    deleted_at: OffsetDateTime,
) -> Result<(), TopicsRepositoryError> {
    sqlx::query(
        "UPDATE push_delivery_intents \
         SET deleted_at = COALESCE(deleted_at, $3), \
             status = 'failed', \
             claim_owner = NULL, \
             lease_expires_at = NULL, \
             failed_at = COALESCE(failed_at, $3), \
             last_error_code = 'source_deleted', \
             source_message_id = NULL, \
             payload = jsonb_set(payload, '{message_id}', 'null'::jsonb, true) \
         WHERE status IN ('pending', 'claimed', 'retryable') \
           AND deleted_at IS NULL \
           AND (source_message_id = ANY($1) OR source_event_id = ANY($2))",
    )
    .bind(message_ids.to_vec())
    .bind(topic_event_ids.to_vec())
    .bind(deleted_at)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(|error| database_error("topic_delete_push_stop", error))
}

async fn hide_notifications(
    connection: &mut PgConnection,
    topic_id: Uuid,
    topic_chatroom_id: Uuid,
    main_chatroom_id: Uuid,
    message_events: &[ScrubbedMessageEvent],
    deleted_at: OffsetDateTime,
) -> Result<(), TopicsRepositoryError> {
    let announcement_cursors = message_events
        .iter()
        .filter_map(|event| (event.1 == main_chatroom_id).then_some(event.2))
        .collect::<Vec<_>>();
    sqlx::query(
        "UPDATE notifications \
         SET deleted_at = COALESCE(deleted_at, $5), \
             read_at = COALESCE(read_at, $5), \
             payload = '{}'::jsonb \
         WHERE deleted_at IS NULL \
           AND ( \
             topic_id = $1 \
             OR conversation_id = $2 \
             OR (conversation_id = $3 AND source_cursor = ANY($4)) \
           )",
    )
    .bind(topic_id)
    .bind(topic_chatroom_id)
    .bind(main_chatroom_id)
    .bind(announcement_cursors)
    .bind(deleted_at)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(|error| database_error("topic_delete_notifications", error))
}

async fn delete_topic_side_tables(
    connection: &mut PgConnection,
    topic_id: Uuid,
    topic_chatroom_id: Uuid,
) -> Result<(), TopicsRepositoryError> {
    sqlx::query(
        "UPDATE chatroom_reads \
         SET deleted_at = COALESCE(deleted_at, clock_timestamp()) \
         WHERE chatroom_id = $1 AND deleted_at IS NULL",
    )
    .bind(topic_chatroom_id)
    .execute(&mut *connection)
    .await
    .map_err(|error| database_error("topic_delete_reads", error))?;
    sqlx::query(
        "UPDATE topic_tags \
         SET deleted_at = COALESCE(deleted_at, clock_timestamp()), \
             tag = 'deleted-' || replace(id::text, '-', ''), \
             confidence = NULL \
         WHERE topic_id = $1 AND deleted_at IS NULL",
    )
    .bind(topic_id)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(|error| database_error("topic_delete_tags", error))
}

async fn scrub_topic_row_and_chatroom(
    connection: &mut PgConnection,
    topic_id: Uuid,
    topic_chatroom_id: Uuid,
    deleted_at: OffsetDateTime,
) -> Result<(), TopicsRepositoryError> {
    sqlx::query(
        "UPDATE topics \
         SET deleted_at = $2, title = '', body = NULL \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(topic_id)
    .bind(deleted_at)
    .execute(&mut *connection)
    .await
    .map_err(|error| database_error("topic_delete_topic_scrub", error))?;
    sqlx::query(
        "UPDATE chatrooms \
         SET deleted_at = COALESCE(deleted_at, $2) \
         WHERE id = $1",
    )
    .bind(topic_chatroom_id)
    .bind(deleted_at)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(|error| database_error("topic_delete_chatroom_scrub", error))
}

async fn append_message_deleted_event(
    connection: &mut PgConnection,
    message_id: Uuid,
    chatroom_id: Uuid,
    group_id: Uuid,
    deleted_by: Uuid,
    deleted_at: OffsetDateTime,
) -> Result<(), TopicsRepositoryError> {
    let event_id = Uuid::new_v4();
    let data = MessageDeletedData {
        message_id,
        chatroom_id,
        group_id,
        deleted_at,
        deleted_by,
        reason: "topic_deleted".to_owned(),
    };
    let payload = serde_json::to_value(&data).map_err(|_| TopicsRepositoryError::InvalidData)?;
    let (cursor, occurred_at) = sqlx::query_as::<_, (i64, OffsetDateTime)>(
        "INSERT INTO conversation_events \
             (id, conversation_id, event_type, event_version, payload) \
         VALUES ($1, $2, 'message.deleted', 1, $3) \
         RETURNING cursor, occurred_at",
    )
    .bind(event_id)
    .bind(chatroom_id)
    .bind(payload)
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| database_error("topic_delete_announcement_event", error))?;
    let event = RealtimeServerEvent::MessageDeleted(MessageDeletedEvent {
        version: 1,
        event_type: MessageDeletedType::MessageDeleted,
        event_id,
        conversation_id: chatroom_id,
        cursor: cursor.to_string(),
        occurred_at,
        data,
    });
    insert_outbox(connection, event_id, chatroom_id, "message.deleted", event).await
}

async fn append_topic_deleted_event(
    connection: &mut PgConnection,
    command: &DeleteTopicCommand,
    topic_chatroom_id: Uuid,
    main_chatroom_id: Uuid,
    announcement_message_id: Option<Uuid>,
    deleted_at: OffsetDateTime,
) -> Result<(), TopicsRepositoryError> {
    let event_id = Uuid::new_v4();
    let data = TopicDeletedData {
        topic_id: command.topic_id,
        topic_chatroom_id,
        group_id: command.group_id,
        deleted_at,
        deleted_by: command.actor_id,
        announcement_message_id,
    };
    let payload = serde_json::to_value(&data).map_err(|_| TopicsRepositoryError::InvalidData)?;
    let (cursor, occurred_at) = sqlx::query_as::<_, (i64, OffsetDateTime)>(
        "INSERT INTO conversation_events \
             (id, conversation_id, event_type, event_version, payload) \
         VALUES ($1, $2, 'topic.deleted', 1, $3) \
         RETURNING cursor, occurred_at",
    )
    .bind(event_id)
    .bind(main_chatroom_id)
    .bind(payload)
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| database_error("topic_delete_event", error))?;
    let event = RealtimeServerEvent::TopicDeleted(TopicDeletedEvent {
        version: 1,
        event_type: TopicDeletedType::TopicDeleted,
        event_id,
        conversation_id: main_chatroom_id,
        cursor: cursor.to_string(),
        occurred_at,
        data,
    });
    insert_outbox(
        connection,
        event_id,
        main_chatroom_id,
        "topic.deleted",
        event,
    )
    .await
}

async fn insert_outbox(
    connection: &mut PgConnection,
    event_id: Uuid,
    conversation_id: Uuid,
    event_type: &'static str,
    event: RealtimeServerEvent,
) -> Result<(), TopicsRepositoryError> {
    let payload = serde_json::to_value(event).map_err(|_| TopicsRepositoryError::InvalidData)?;
    sqlx::query(
        "INSERT INTO outbox_events \
             (id, intent_type, event_type, event_version, aggregate_type, aggregate_id, \
              conversation_event_id, payload) \
         VALUES ($1, 'conversation', $2, 1, 'conversation', $3, $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(event_type)
    .bind(conversation_id)
    .bind(event_id)
    .bind(payload)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(|error| database_error("topic_delete_outbox", error))
}
