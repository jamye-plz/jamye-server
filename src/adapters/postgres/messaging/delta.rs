use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    domain::messaging::{
        CanonicalMessage, ConversationEvent, DeltaItem, EventPage, MessageCreatedEvent,
        MessageCreatedType, MessageDeletedData, MessageDeletedEvent, MessageDeletedType,
        ReconcileScope, TopicDeletedData, TopicDeletedEvent, TopicDeletedType,
        UnsupportedEventMarker,
    },
    ports::messaging::{ContractProjection, DeltaQuery, MessagingRepositoryError},
};

type DeltaRow = (
    bool,
    Option<Uuid>,
    Option<i64>,
    Option<String>,
    Option<i16>,
    Option<Value>,
    Option<OffsetDateTime>,
    Option<Uuid>,
    Option<String>,
);

struct DeltaEvent {
    event: ConversationEvent,
    feed_group_id: Uuid,
    feed_chatroom_type: String,
}

pub(super) async fn page(
    pool: &PgPool,
    query: DeltaQuery,
) -> Result<EventPage, MessagingRepositoryError> {
    let rows = fetch_rows(pool, &query).await?;
    if rows.first().is_some_and(|row| !row.0) {
        return Err(MessagingRepositoryError::MembershipRequired);
    }
    let mut events = rows
        .into_iter()
        .filter_map(|row| delta_event(row, query.conversation_id))
        .collect::<Vec<_>>();
    let page_limit = usize::try_from(query.limit).map_err(|_| database_error("limit"))?;
    let has_more = events.len() > page_limit;
    events.truncate(page_limit);
    let items = events
        .into_iter()
        .map(|event| project_event(event, query.projection))
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = if has_more {
        items.last().map(DeltaItem::cursor).map(str::to_owned)
    } else {
        None
    };
    Ok(EventPage { items, next_cursor })
}

async fn fetch_rows(
    pool: &PgPool,
    query: &DeltaQuery,
) -> Result<Vec<DeltaRow>, MessagingRepositoryError> {
    let fetch_limit = i64::from(query.limit) + 1;
    sqlx::query_as::<_, DeltaRow>(
        "WITH membership_access AS MATERIALIZED ( \
             SELECT EXISTS ( \
                 SELECT 1 FROM chatrooms c \
                 LEFT JOIN topics topic ON topic.id = c.topic_id \
                 JOIN groups g ON g.id = c.group_id AND g.deleted_at IS NULL \
                 JOIN memberships m ON m.group_id = g.id AND m.user_id = $2 \
                   AND m.deleted_at IS NULL \
                 JOIN users account ON account.id = m.user_id \
                 WHERE c.id = $1 \
                   AND c.deleted_at IS NULL \
                   AND (c.topic_id IS NULL OR topic.deleted_at IS NULL) \
                   AND account.deleted_at IS NULL \
             ) AS allowed, \
             (SELECT c.group_id FROM chatrooms c WHERE c.id = $1) AS feed_group_id, \
             (SELECT c.type::TEXT FROM chatrooms c WHERE c.id = $1) AS feed_chatroom_type \
         ), page AS ( \
             SELECT e.id, e.cursor, e.event_type, e.event_version, \
                    CASE \
                        WHEN e.event_type = 'message.created' \
                         AND e.event_version = 1 \
                         AND sender.deleted_at IS NOT NULL \
                        THEN jsonb_set( \
                            jsonb_set( \
                                e.payload, \
                                '{sender_nickname}', \
                                to_jsonb('탈퇴한 사용자'::TEXT), \
                                true \
                            ), \
                            '{sender_avatar_url}', \
                            'null'::jsonb, \
                            true \
                        ) \
                        ELSE e.payload \
                    END, \
                    e.occurred_at \
             FROM conversation_events e \
             LEFT JOIN users sender ON sender.id::TEXT = e.payload ->> 'sender_id' \
             WHERE e.conversation_id = $1 \
               AND e.cursor > COALESCE($3::BIGINT, 0::BIGINT) \
               AND (SELECT allowed FROM membership_access) \
             ORDER BY e.cursor ASC \
             LIMIT $4 \
         ) \
         SELECT a.allowed, p.id, p.cursor, p.event_type, p.event_version, p.payload, p.occurred_at, \
                a.feed_group_id, a.feed_chatroom_type \
         FROM membership_access a \
         LEFT JOIN page p ON TRUE \
         ORDER BY p.cursor ASC NULLS LAST",
    )
    .bind(query.conversation_id)
    .bind(query.user_id)
    .bind(query.after)
    .bind(fetch_limit)
    .fetch_all(pool)
    .await
    .map_err(|_| database_error("delta_page"))
}

fn delta_event(row: DeltaRow, conversation_id: Uuid) -> Option<DeltaEvent> {
    Some(DeltaEvent {
        event: ConversationEvent {
            id: row.1?,
            cursor: row.2?,
            conversation_id,
            event_type: row.3?,
            event_version: row.4?,
            payload: row.5?,
            occurred_at: row.6?,
        },
        feed_group_id: row.7?,
        feed_chatroom_type: row.8?,
    })
}

fn project_event(
    event: DeltaEvent,
    projection: ContractProjection,
) -> Result<DeltaItem, MessagingRepositoryError> {
    match projection {
        ContractProjection::Current => project_v2_event(event),
        ContractProjection::Previous => project_v1_event(event.event),
    }
}

fn project_v2_event(event: DeltaEvent) -> Result<DeltaItem, MessagingRepositoryError> {
    let DeltaEvent {
        event,
        feed_group_id,
        feed_chatroom_type,
    } = event;
    if event.event_type == "message.deleted"
        && event.event_version == 1
        && let Ok(data) = serde_json::from_value::<MessageDeletedData>(event.payload.clone())
        && data.chatroom_id == event.conversation_id
    {
        return Ok(DeltaItem::MessageDeleted(MessageDeletedEvent {
            version: 1,
            event_type: MessageDeletedType::MessageDeleted,
            event_id: event.id,
            conversation_id: event.conversation_id,
            cursor: event.cursor.to_string(),
            occurred_at: event.occurred_at,
            data,
        }));
    }
    if event.event_type == "topic.deleted"
        && event.event_version == 1
        && let Ok(data) = serde_json::from_value::<TopicDeletedData>(event.payload.clone())
        && data.group_id == feed_group_id
        && feed_chatroom_type == "main"
    {
        return Ok(DeltaItem::TopicDeleted(TopicDeletedEvent {
            version: 1,
            event_type: TopicDeletedType::TopicDeleted,
            event_id: event.id,
            conversation_id: event.conversation_id,
            cursor: event.cursor.to_string(),
            occurred_at: event.occurred_at,
            data,
        }));
    }
    project_v1_event(event)
}

fn project_v1_event(event: ConversationEvent) -> Result<DeltaItem, MessagingRepositoryError> {
    if event.event_type == "message.created"
        && event.event_version == 1
        && let Ok(message) = serde_json::from_value::<CanonicalMessage>(event.payload.clone())
        && message.chatroom_id == event.conversation_id
    {
        return Ok(DeltaItem::Known(MessageCreatedEvent {
            version: 1,
            event_type: MessageCreatedType::MessageCreated,
            event_id: event.id,
            conversation_id: message.chatroom_id,
            cursor: event.cursor.to_string(),
            occurred_at: event.occurred_at,
            data: message,
        }));
    }
    let reconcile_scope = safe_reconcile_scope(&event)?;
    Ok(DeltaItem::Unsupported(UnsupportedEventMarker {
        event_id: event.id,
        cursor: event.cursor.to_string(),
        reconcile_scope,
    }))
}

fn safe_reconcile_scope(
    event: &ConversationEvent,
) -> Result<ReconcileScope, MessagingRepositoryError> {
    if event.event_type.starts_with("message.") {
        return Ok(ReconcileScope::ChatHistory);
    }
    if event.event_type.starts_with("topic.") {
        return Ok(ReconcileScope::GroupTopics);
    }
    if event.event_type.starts_with("notification.") {
        return Ok(ReconcileScope::Notifications);
    }
    match event.payload.get("reconcile_scope").and_then(Value::as_str) {
        Some("chat_history") => Ok(ReconcileScope::ChatHistory),
        Some("group_topics") => Ok(ReconcileScope::GroupTopics),
        Some("notifications") => Ok(ReconcileScope::Notifications),
        _ => Err(MessagingRepositoryError::ContractUpgradeRequired),
    }
}

fn database_error(operation: &'static str) -> MessagingRepositoryError {
    tracing::warn!(
        dependency = "postgres",
        failure_kind = "delta_read",
        operation,
        "PostgreSQL delta read failed"
    );
    MessagingRepositoryError::DatabaseUnavailable
}
