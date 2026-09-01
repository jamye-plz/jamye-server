use serde::Serialize;
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    domain::messaging::{CanonicalMessage, MessageCreatedEvent, MessageCreatedType, MessageKind},
    ports::topics::{
        CreateTopicCommand, CreateTopicOutcome, PatchTopicCommand, ReplaceTopicTagsCommand,
        TopicRecord, TopicStatus, TopicTagPage, TopicTagRecord, TopicsRepositoryError,
    },
};

use super::{
    database_error,
    query::{
        TopicBaseRow, TopicMediaRow, TopicTagRow, media_from_row, tag_from_row, topic_from_row,
    },
};

type MembershipRow = (String, String, Option<String>);

#[derive(Clone, Debug, Serialize)]
struct TopicCreatedData {
    topic_id: Uuid,
    group_id: Uuid,
    chatroom_id: Uuid,
    author_id: Uuid,
    title: String,
}

#[derive(Serialize)]
struct TopicCreatedEvent {
    version: u8,
    #[serde(rename = "type")]
    event_type: &'static str,
    event_id: Uuid,
    conversation_id: Uuid,
    cursor: String,
    #[serde(with = "time::serde::rfc3339")]
    occurred_at: OffsetDateTime,
    data: TopicCreatedData,
}

pub(super) async fn create_topic(
    connection: &mut PgConnection,
    command: &CreateTopicCommand,
) -> Result<CreateTopicOutcome, TopicsRepositoryError> {
    let membership =
        lock_group_and_membership(connection, command.group_id, command.author_id).await?;
    let inserted = sqlx::query_as::<_, (OffsetDateTime, OffsetDateTime)>(
        "INSERT INTO topics \
             (id, group_id, author_id, idempotency_key, request_fingerprint, title) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (author_id, idempotency_key) DO NOTHING \
         RETURNING created_at, updated_at",
    )
    .bind(command.topic_id)
    .bind(command.group_id)
    .bind(command.author_id)
    .bind(command.idempotency_key)
    .bind(&command.request_fingerprint)
    .bind(&command.title)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| database_error("topic_insert", error))?;
    let Some((created_at, updated_at)) = inserted else {
        return existing_topic(connection, command).await;
    };

    sqlx::query(
        "INSERT INTO chatrooms (id, group_id, type, topic_id) \
         VALUES ($1, $2, 'topic', $3)",
    )
    .bind(command.topic_chatroom_id)
    .bind(command.group_id)
    .bind(command.topic_id)
    .execute(&mut *connection)
    .await
    .map_err(|error| database_error("topic_chatroom_insert", error))?;

    let topic_data = TopicCreatedData {
        topic_id: command.topic_id,
        group_id: command.group_id,
        chatroom_id: command.topic_chatroom_id,
        author_id: command.author_id,
        title: command.title.clone(),
    };
    let topic_payload =
        serde_json::to_value(&topic_data).map_err(|_| TopicsRepositoryError::InvalidData)?;
    let (topic_cursor, topic_occurred_at) = sqlx::query_as::<_, (i64, OffsetDateTime)>(
        "INSERT INTO conversation_events \
             (id, conversation_id, event_type, event_version, payload) \
         VALUES ($1, $2, 'topic.created', 1, $3) \
         RETURNING cursor, occurred_at",
    )
    .bind(command.topic_event_id)
    .bind(command.topic_chatroom_id)
    .bind(topic_payload)
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| database_error("topic_event_insert", error))?;
    let topic_event = TopicCreatedEvent {
        version: 1,
        event_type: "topic.created",
        event_id: command.topic_event_id,
        conversation_id: command.topic_chatroom_id,
        cursor: topic_cursor.to_string(),
        occurred_at: topic_occurred_at,
        data: topic_data,
    };
    insert_outbox(
        connection,
        command.topic_outbox_id,
        command.topic_chatroom_id,
        command.topic_event_id,
        "topic.created",
        serde_json::to_value(topic_event).map_err(|_| TopicsRepositoryError::InvalidData)?,
    )
    .await?;

    sqlx::query(
        "INSERT INTO chatroom_reads \
             (id, user_id, chatroom_id, last_read_cursor) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(command.author_read_marker_id)
    .bind(command.author_id)
    .bind(command.topic_chatroom_id)
    .bind(topic_cursor)
    .execute(&mut *connection)
    .await
    .map_err(|error| database_error("topic_author_read_insert", error))?;

    let main_chatroom_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM chatrooms WHERE group_id = $1 AND type = 'main'",
    )
    .bind(command.group_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| database_error("topic_main_chatroom", error))?
    .ok_or(TopicsRepositoryError::InvalidData)?;
    let announcement_created_at = sqlx::query_scalar::<_, OffsetDateTime>(
        "INSERT INTO messages \
             (id, chatroom_id, sender_id, client_msg_id, body, type) \
         VALUES ($1, $2, $3, $4, $5, 'user') \
         RETURNING created_at",
    )
    .bind(command.announcement_message_id)
    .bind(main_chatroom_id)
    .bind(command.author_id)
    .bind(command.announcement_client_msg_id)
    .bind(&command.announcement_body)
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| database_error("topic_announcement_insert", error))?;
    let announcement = CanonicalMessage {
        id: command.announcement_message_id,
        chatroom_id: main_chatroom_id,
        sender_id: Some(command.author_id),
        client_msg_id: Some(command.announcement_client_msg_id),
        body: Some(command.announcement_body.clone()),
        message_type: MessageKind::User,
        created_at: announcement_created_at,
        media: Vec::new(),
    };
    let announcement_payload =
        serde_json::to_value(&announcement).map_err(|_| TopicsRepositoryError::InvalidData)?;
    let (announcement_cursor, announcement_occurred_at) =
        sqlx::query_as::<_, (i64, OffsetDateTime)>(
            "INSERT INTO conversation_events \
                 (id, conversation_id, event_type, event_version, payload) \
             VALUES ($1, $2, 'message.created', 1, $3) \
             RETURNING cursor, occurred_at",
        )
        .bind(command.announcement_event_id)
        .bind(main_chatroom_id)
        .bind(announcement_payload)
        .fetch_one(&mut *connection)
        .await
        .map_err(|error| database_error("topic_announcement_event_insert", error))?;
    let announcement_event = MessageCreatedEvent {
        version: 1,
        event_type: MessageCreatedType::MessageCreated,
        event_id: command.announcement_event_id,
        conversation_id: main_chatroom_id,
        cursor: announcement_cursor.to_string(),
        occurred_at: announcement_occurred_at,
        data: announcement,
    };
    insert_outbox(
        connection,
        command.announcement_outbox_id,
        main_chatroom_id,
        command.announcement_event_id,
        "message.created",
        serde_json::to_value(announcement_event).map_err(|_| TopicsRepositoryError::InvalidData)?,
    )
    .await?;

    Ok(CreateTopicOutcome::Created(TopicRecord {
        id: command.topic_id,
        group_id: command.group_id,
        author_id: command.author_id,
        author_nickname: membership.1,
        author_avatar_url: membership.2,
        title: command.title.clone(),
        body: None,
        status: TopicStatus::Seed,
        tags: Vec::new(),
        media: Vec::new(),
        chatroom_id: command.topic_chatroom_id,
        unread: false,
        created_at,
        updated_at,
    }))
}

pub(super) async fn notification_context(
    connection: &mut PgConnection,
    topic: &TopicRecord,
) -> Result<crate::ports::topics::TopicNotificationContext, TopicsRepositoryError> {
    let rows = sqlx::query_as::<_, (Uuid, Uuid, Uuid, Uuid, String)>(
        "SELECT topic.group_id, topic.id, chatroom.id, event.id, author.nickname \
         FROM topics AS topic \
         JOIN chatrooms AS chatroom ON chatroom.topic_id = topic.id \
           AND chatroom.group_id = topic.group_id AND chatroom.type = 'topic' \
         JOIN conversation_events AS event ON event.conversation_id = chatroom.id \
           AND event.event_type = 'topic.created' AND event.event_version = 1 \
           AND event.payload ->> 'topic_id' = topic.id::text \
         JOIN users AS author ON author.id = topic.author_id \
         WHERE topic.id = $1 AND topic.group_id = $2 AND topic.author_id = $3 \
         FOR SHARE OF topic, chatroom, event, author",
    )
    .bind(topic.id)
    .bind(topic.group_id)
    .bind(topic.author_id)
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| database_error("topic_notification_context", error))?;
    let [row] = rows.as_slice() else {
        return Err(TopicsRepositoryError::InvalidData);
    };
    Ok(crate::ports::topics::TopicNotificationContext {
        group_id: row.0,
        topic_id: row.1,
        conversation_id: row.2,
        source_event_id: row.3,
        author_id: topic.author_id,
        author_display_name: row.4.clone(),
    })
}

pub(super) async fn patch_topic(
    connection: &mut PgConnection,
    command: &PatchTopicCommand,
) -> Result<TopicRecord, TopicsRepositoryError> {
    lock_group_and_membership(connection, command.group_id, command.actor_id).await?;
    let author_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT author_id FROM topics WHERE id = $1 AND group_id = $2 FOR UPDATE",
    )
    .bind(command.topic_id)
    .bind(command.group_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| database_error("topic_patch_authorize", error))?;
    if author_id != Some(command.actor_id) {
        return Err(TopicsRepositoryError::AuthorRequired);
    }
    sqlx::query(
        "UPDATE topics SET \
             title = COALESCE($3, title), \
             body = COALESCE($4, body), \
             status = CASE WHEN $4::text IS NOT NULL THEN 'enriched' ELSE status END, \
             updated_at = CASE \
                 WHEN $3::text IS NOT NULL OR $4::text IS NOT NULL \
                 THEN clock_timestamp() ELSE updated_at \
             END \
         WHERE id = $1 AND group_id = $2",
    )
    .bind(command.topic_id)
    .bind(command.group_id)
    .bind(&command.title)
    .bind(&command.body)
    .execute(&mut *connection)
    .await
    .map_err(|error| database_error("topic_patch", error))?;
    load_topic(connection, command.topic_id, command.actor_id).await
}

pub(super) async fn promote_enriched(
    connection: &mut PgConnection,
    topic_id: Uuid,
) -> Result<TopicStatus, TopicsRepositoryError> {
    let status = sqlx::query_scalar::<_, String>(
        "UPDATE topics SET \
             status = 'enriched', \
             updated_at = CASE \
                 WHEN status = 'seed' THEN clock_timestamp() ELSE updated_at \
             END \
         WHERE id = $1 \
         RETURNING status",
    )
    .bind(topic_id)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("topic_promote_enriched", error))?
    .ok_or(TopicsRepositoryError::TopicNotFound)?;
    TopicStatus::parse(&status).ok_or(TopicsRepositoryError::InvalidData)
}

pub(super) async fn replace_tags(
    connection: &mut PgConnection,
    command: &ReplaceTopicTagsCommand,
) -> Result<TopicTagPage, TopicsRepositoryError> {
    let membership =
        lock_group_and_membership(connection, command.group_id, command.actor_id).await?;
    let author_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT author_id FROM topics WHERE id = $1 AND group_id = $2 FOR UPDATE",
    )
    .bind(command.topic_id)
    .bind(command.group_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| database_error("topic_tag_authorize", error))?
    .ok_or(TopicsRepositoryError::TopicNotFound)?;
    if author_id != command.actor_id && membership.0 != "owner" {
        return Err(TopicsRepositoryError::TopicManageRequired);
    }
    sqlx::query("DELETE FROM topic_tags WHERE topic_id = $1")
        .bind(command.topic_id)
        .execute(&mut *connection)
        .await
        .map_err(|error| database_error("topic_tag_delete", error))?;
    let mut items = Vec::with_capacity(command.tags.len());
    for tag in &command.tags {
        sqlx::query(
            "INSERT INTO topic_tags (id, topic_id, tag, source, confidence) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(tag.id)
        .bind(command.topic_id)
        .bind(&tag.tag)
        .bind(tag.source.as_str())
        .bind(tag.confidence)
        .execute(&mut *connection)
        .await
        .map_err(|error| database_error("topic_tag_insert", error))?;
        items.push(TopicTagRecord {
            id: tag.id,
            topic_id: command.topic_id,
            tag: tag.tag.clone(),
            source: tag.source,
            confidence: tag.confidence,
        });
    }
    Ok(TopicTagPage {
        items,
        next_cursor: None,
    })
}

async fn lock_group_and_membership(
    connection: &mut PgConnection,
    group_id: Uuid,
    actor_id: Uuid,
) -> Result<MembershipRow, TopicsRepositoryError> {
    let live = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM groups \
         WHERE id = $1 AND deleted_at IS NULL \
         FOR UPDATE",
    )
    .bind(group_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| database_error("topic_group_lock", error))?;
    if live.is_none() {
        return Err(TopicsRepositoryError::GroupNotFound);
    }
    sqlx::query_as::<_, MembershipRow>(
        "SELECT membership.role, actor.nickname, actor.avatar_url \
         FROM memberships membership \
         JOIN users actor ON actor.id = membership.user_id \
         WHERE membership.group_id = $1 AND membership.user_id = $2",
    )
    .bind(group_id)
    .bind(actor_id)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("topic_membership", error))?
    .ok_or(TopicsRepositoryError::MembershipRequired)
}

async fn existing_topic(
    connection: &mut PgConnection,
    command: &CreateTopicCommand,
) -> Result<CreateTopicOutcome, TopicsRepositoryError> {
    let existing = sqlx::query_as::<_, (Uuid, Uuid, String)>(
        "SELECT id, group_id, request_fingerprint \
         FROM topics WHERE author_id = $1 AND idempotency_key = $2",
    )
    .bind(command.author_id)
    .bind(command.idempotency_key)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| database_error("topic_idempotency_read", error))?
    .ok_or(TopicsRepositoryError::InvalidData)?;
    if existing.1 != command.group_id || existing.2 != command.request_fingerprint {
        return Err(TopicsRepositoryError::IdempotencyConflict);
    }
    load_topic(connection, existing.0, command.author_id)
        .await
        .map(CreateTopicOutcome::Existing)
}

async fn load_topic(
    connection: &mut PgConnection,
    topic_id: Uuid,
    actor_id: Uuid,
) -> Result<TopicRecord, TopicsRepositoryError> {
    let row = sqlx::query_as::<_, TopicBaseRow>(
        "SELECT t.id, t.group_id, t.author_id, t.title, t.body, t.status, \
                t.created_at, t.updated_at, topic_chat.id, author.nickname, \
                author.avatar_url, \
                EXISTS ( \
                    SELECT 1 FROM conversation_events event \
                    WHERE event.conversation_id = topic_chat.id \
                      AND event.cursor > COALESCE(( \
                          SELECT marker.last_read_cursor FROM chatroom_reads marker \
                          WHERE marker.user_id = $2 AND marker.chatroom_id = topic_chat.id \
                      ), 0) \
                ) AS unread \
         FROM topics t \
         JOIN chatrooms topic_chat \
           ON topic_chat.topic_id = t.id AND topic_chat.type = 'topic' \
         JOIN users author ON author.id = t.author_id \
         WHERE t.id = $1",
    )
    .bind(topic_id)
    .bind(actor_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| database_error("topic_transaction_get", error))?
    .ok_or(TopicsRepositoryError::TopicNotFound)?;
    let mut topic = topic_from_row(row)?;
    let tags = sqlx::query_as::<_, TopicTagRow>(
        "SELECT id, topic_id, tag, source, confidence \
         FROM topic_tags WHERE topic_id = $1 ORDER BY tag, id",
    )
    .bind(topic_id)
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| database_error("topic_transaction_tags", error))?;
    topic.tags = tags
        .into_iter()
        .map(tag_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    let media = sqlx::query_as::<_, TopicMediaRow>(
        "SELECT id, topic_id, media_upload_id, type, object_key, width, height, byte_size, created_at \
         FROM topic_media WHERE topic_id = $1 ORDER BY created_at, id",
    )
    .bind(topic_id)
    .fetch_all(connection)
    .await
    .map_err(|error| database_error("topic_transaction_media", error))?;
    topic.media = media.into_iter().map(media_from_row).collect();
    Ok(topic)
}

async fn insert_outbox(
    connection: &mut PgConnection,
    outbox_id: Uuid,
    conversation_id: Uuid,
    event_id: Uuid,
    event_type: &'static str,
    payload: serde_json::Value,
) -> Result<(), TopicsRepositoryError> {
    sqlx::query(
        "INSERT INTO outbox_events \
             (id, intent_type, event_type, event_version, aggregate_type, aggregate_id, \
              conversation_event_id, payload) \
         VALUES ($1, 'conversation', $2, 1, 'conversation', $3, $4, $5)",
    )
    .bind(outbox_id)
    .bind(event_type)
    .bind(conversation_id)
    .bind(event_id)
    .bind(payload)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(|error| database_error("topic_outbox_insert", error))
}
