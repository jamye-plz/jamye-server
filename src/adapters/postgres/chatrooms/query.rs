use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    domain::messaging::{CanonicalMessage, MessageKind},
    ports::chatrooms::{
        ChatroomKind, ChatroomPage, ChatroomRecord, ChatroomsRepositoryError, ListChatroomsQuery,
        MessageHistoryPage, MessageHistoryQuery, MessageHistoryRecord, ReadMarker, ReadMarkerQuery,
    },
};

use super::database_error;

type ChatroomAccessRow = (
    bool,
    bool,
    Option<Uuid>,
    Option<Uuid>,
    Option<String>,
    Option<Uuid>,
    Option<OffsetDateTime>,
);

type MessageAccessRow = (
    bool,
    bool,
    Option<Uuid>,
    Option<Uuid>,
    Option<Uuid>,
    Option<Uuid>,
    Option<String>,
    Option<String>,
    Option<OffsetDateTime>,
    Option<String>,
    Option<String>,
);

type ReadMarkerAccessRow = (
    bool,
    Option<Uuid>,
    Option<Uuid>,
    Option<Uuid>,
    Option<i64>,
    Option<OffsetDateTime>,
);

pub(super) async fn list_chatrooms(
    pool: &PgPool,
    query: ListChatroomsQuery,
) -> Result<ChatroomPage, ChatroomsRepositoryError> {
    let fetch_limit = i64::from(query.limit) + 1;
    let rows = sqlx::query_as::<_, ChatroomAccessRow>(
        "WITH actor_access AS ( \
             SELECT \
                 EXISTS ( \
                     SELECT 1 FROM groups g \
                     JOIN memberships actor_membership \
                       ON actor_membership.group_id = g.id \
                      AND actor_membership.user_id = $2 \
                     WHERE g.id = $1 AND g.deleted_at IS NULL \
                 ) AS member, \
                 ( \
                     $3::uuid IS NULL \
                     OR EXISTS ( \
                         SELECT 1 FROM chatrooms cursor_chatroom \
                         WHERE cursor_chatroom.id = $3 \
                           AND cursor_chatroom.group_id = $1 \
                     ) \
                 ) AS cursor_valid \
         ), page AS ( \
             SELECT c.id, c.group_id, c.type, c.topic_id, c.created_at \
             FROM chatrooms c \
             CROSS JOIN actor_access \
             WHERE c.group_id = $1 AND actor_access.member AND actor_access.cursor_valid \
               AND ( \
                 $3::uuid IS NULL \
                 OR (c.created_at, c.id) > ( \
                     SELECT cursor_chatroom.created_at, cursor_chatroom.id \
                     FROM chatrooms cursor_chatroom \
                     WHERE cursor_chatroom.id = $3 \
                       AND cursor_chatroom.group_id = $1 \
                 ) \
               ) \
             ORDER BY c.created_at, c.id \
             LIMIT $4 \
         ) \
         SELECT actor_access.member, actor_access.cursor_valid, page.id, page.group_id, \
                page.type, page.topic_id, page.created_at \
         FROM actor_access LEFT JOIN page ON TRUE \
         ORDER BY page.created_at, page.id",
    )
    .bind(query.group_id)
    .bind(query.user_id)
    .bind(query.after)
    .bind(fetch_limit)
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("chatroom_list", error))?;

    let first = rows.first().ok_or(ChatroomsRepositoryError::Unavailable)?;
    require_access(first.0, first.1)?;
    let mut items = rows
        .into_iter()
        .filter_map(chatroom_from_access_row)
        .collect::<Result<Vec<_>, _>>()?;
    let has_more = items.len() > query.limit as usize;
    if has_more {
        items.truncate(query.limit as usize);
    }
    let next_cursor = has_more
        .then(|| items.last().map(|chatroom| chatroom.id.to_string()))
        .flatten();
    Ok(ChatroomPage { items, next_cursor })
}

pub(super) async fn message_history(
    pool: &PgPool,
    query: MessageHistoryQuery,
) -> Result<MessageHistoryPage, ChatroomsRepositoryError> {
    let fetch_limit = i64::from(query.limit) + 1;
    let rows = sqlx::query_as::<_, MessageAccessRow>(
        "WITH actor_access AS ( \
             SELECT \
                 EXISTS ( \
                     SELECT 1 FROM chatrooms c \
                     JOIN groups g ON g.id = c.group_id AND g.deleted_at IS NULL \
                     JOIN memberships actor_membership \
                       ON actor_membership.group_id = g.id \
                      AND actor_membership.user_id = $2 \
                     WHERE c.id = $1 \
                 ) AS member, \
                 ( \
                     $3::uuid IS NULL \
                     OR EXISTS ( \
                         SELECT 1 FROM messages cursor_message \
                         WHERE cursor_message.id = $3 \
                           AND cursor_message.chatroom_id = $1 \
                     ) \
                 ) AS cursor_valid \
         ), page AS ( \
             SELECT m.id, m.chatroom_id, m.sender_id, m.client_msg_id, m.body, m.type, \
                    m.created_at, sender.nickname, sender.avatar_url \
             FROM messages m \
             LEFT JOIN users sender ON sender.id = m.sender_id \
             CROSS JOIN actor_access \
             WHERE m.chatroom_id = $1 AND actor_access.member AND actor_access.cursor_valid \
               AND ( \
                 $3::uuid IS NULL \
                 OR (m.created_at, m.id) < ( \
                     SELECT cursor_message.created_at, cursor_message.id \
                     FROM messages cursor_message \
                     WHERE cursor_message.id = $3 \
                       AND cursor_message.chatroom_id = $1 \
                 ) \
               ) \
             ORDER BY m.created_at DESC, m.id DESC \
             LIMIT $4 \
         ) \
         SELECT actor_access.member, actor_access.cursor_valid, page.id, page.chatroom_id, \
                page.sender_id, page.client_msg_id, page.body, page.type, page.created_at, \
                page.nickname, page.avatar_url \
         FROM actor_access LEFT JOIN page ON TRUE \
         ORDER BY page.created_at DESC, page.id DESC",
    )
    .bind(query.chatroom_id)
    .bind(query.user_id)
    .bind(query.before)
    .bind(fetch_limit)
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("message_history", error))?;

    let first = rows.first().ok_or(ChatroomsRepositoryError::Unavailable)?;
    require_access(first.0, first.1)?;
    let mut items = rows
        .into_iter()
        .filter_map(message_from_access_row)
        .collect::<Result<Vec<_>, _>>()?;
    let has_more = items.len() > query.limit as usize;
    if has_more {
        items.truncate(query.limit as usize);
    }
    let next_cursor = has_more
        .then(|| items.last().map(|item| item.message.id.to_string()))
        .flatten();
    items.reverse();
    Ok(MessageHistoryPage { items, next_cursor })
}

pub(super) async fn read_marker(
    pool: &PgPool,
    query: ReadMarkerQuery,
) -> Result<Option<ReadMarker>, ChatroomsRepositoryError> {
    let row = sqlx::query_as::<_, ReadMarkerAccessRow>(
        "WITH actor_access AS ( \
             SELECT EXISTS ( \
                 SELECT 1 FROM chatrooms c \
                 JOIN groups g ON g.id = c.group_id AND g.deleted_at IS NULL \
                 JOIN memberships actor_membership \
                   ON actor_membership.group_id = g.id \
                  AND actor_membership.user_id = $2 \
                 WHERE c.id = $1 \
             ) AS member \
         ) \
         SELECT actor_access.member, marker.id, marker.user_id, marker.chatroom_id, \
                marker.last_read_cursor, marker.updated_at \
         FROM actor_access \
         LEFT JOIN chatroom_reads marker \
           ON marker.chatroom_id = $1 AND marker.user_id = $2 AND actor_access.member",
    )
    .bind(query.chatroom_id)
    .bind(query.user_id)
    .fetch_one(pool)
    .await
    .map_err(|error| database_error("read_marker_get", error))?;
    if !row.0 {
        return Err(ChatroomsRepositoryError::MembershipRequired);
    }
    let Some(id) = row.1 else {
        return Ok(None);
    };
    Ok(Some(ReadMarker {
        id,
        user_id: row.2.ok_or(ChatroomsRepositoryError::InvalidData)?,
        chatroom_id: row.3.ok_or(ChatroomsRepositoryError::InvalidData)?,
        last_read_cursor: row.4.ok_or(ChatroomsRepositoryError::InvalidData)?,
        updated_at: row.5.ok_or(ChatroomsRepositoryError::InvalidData)?,
    }))
}

fn require_access(member: bool, cursor_valid: bool) -> Result<(), ChatroomsRepositoryError> {
    if !member {
        return Err(ChatroomsRepositoryError::MembershipRequired);
    }
    if !cursor_valid {
        return Err(ChatroomsRepositoryError::CursorInvalid);
    }
    Ok(())
}

fn chatroom_from_access_row(
    row: ChatroomAccessRow,
) -> Option<Result<ChatroomRecord, ChatroomsRepositoryError>> {
    let id = row.2?;
    Some((|| {
        let chatroom_type = row
            .4
            .as_deref()
            .and_then(ChatroomKind::parse)
            .ok_or(ChatroomsRepositoryError::InvalidData)?;
        let topic_id = row.5;
        if (chatroom_type == ChatroomKind::Main && topic_id.is_some())
            || (chatroom_type == ChatroomKind::Topic && topic_id.is_none())
        {
            return Err(ChatroomsRepositoryError::InvalidData);
        }
        Ok(ChatroomRecord {
            id,
            group_id: row.3.ok_or(ChatroomsRepositoryError::InvalidData)?,
            chatroom_type,
            topic_id,
            created_at: row.6.ok_or(ChatroomsRepositoryError::InvalidData)?,
        })
    })())
}

fn message_from_access_row(
    row: MessageAccessRow,
) -> Option<Result<MessageHistoryRecord, ChatroomsRepositoryError>> {
    let id = row.2?;
    Some((|| {
        let message_type = match row.7.as_deref() {
            Some("user") => MessageKind::User,
            Some("system") => MessageKind::System,
            _ => return Err(ChatroomsRepositoryError::InvalidData),
        };
        let sender_id = row.4;
        let client_msg_id = row.5;
        let sender_nickname = row.9;
        let sender_avatar_url = row.10;
        match message_type {
            MessageKind::User
                if sender_id.is_none() || client_msg_id.is_none() || sender_nickname.is_none() =>
            {
                return Err(ChatroomsRepositoryError::InvalidData);
            }
            MessageKind::System
                if sender_id.is_some()
                    || client_msg_id.is_some()
                    || sender_nickname.is_some()
                    || sender_avatar_url.is_some() =>
            {
                return Err(ChatroomsRepositoryError::InvalidData);
            }
            _ => {}
        }
        Ok(MessageHistoryRecord {
            message: CanonicalMessage {
                id,
                chatroom_id: row.3.ok_or(ChatroomsRepositoryError::InvalidData)?,
                sender_id,
                client_msg_id,
                body: row.6,
                message_type,
                created_at: row.8.ok_or(ChatroomsRepositoryError::InvalidData)?,
                media: Vec::new(),
            },
            sender_nickname,
            sender_avatar_url,
        })
    })())
}
