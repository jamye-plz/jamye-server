use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::ports::chatrooms::{
    ChatroomsRepositoryError, MarkReadCommand, ReadMarker, ReadMarkerAnchor,
};

use super::database_error;

type ReadMarkerRow = (Uuid, Uuid, Uuid, i64, OffsetDateTime);

pub(super) async fn mark_read(
    connection: &mut PgConnection,
    command: &MarkReadCommand,
) -> Result<ReadMarker, ChatroomsRepositoryError> {
    let authorized = sqlx::query_scalar::<_, Uuid>(
        "SELECT c.id \
         FROM chatrooms c \
         JOIN groups g ON g.id = c.group_id AND g.deleted_at IS NULL \
         JOIN memberships actor_membership \
           ON actor_membership.group_id = g.id \
          AND actor_membership.user_id = $2 \
         WHERE c.id = $1 \
         FOR SHARE OF c, g, actor_membership",
    )
    .bind(command.chatroom_id)
    .bind(command.user_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| database_error("read_authorize", error))?;
    if authorized.is_none() {
        return Err(ChatroomsRepositoryError::MembershipRequired);
    }

    let cursor = match command.anchor {
        ReadMarkerAnchor::Cursor(cursor) => {
            cursor_for_existing_cursor(connection, command, cursor).await?
        }
        ReadMarkerAnchor::MessageId(message_id) => {
            cursor_for_message_anchor(connection, command, message_id).await?
        }
    };

    let row = sqlx::query_as::<_, ReadMarkerRow>(
        "INSERT INTO chatroom_reads \
             (id, user_id, chatroom_id, last_read_cursor) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT ON CONSTRAINT uq_chatroom_reads_user_chatroom \
         DO UPDATE SET \
             last_read_cursor = GREATEST( \
                 chatroom_reads.last_read_cursor, EXCLUDED.last_read_cursor \
             ), \
             updated_at = CASE \
                 WHEN EXCLUDED.last_read_cursor > chatroom_reads.last_read_cursor \
                 THEN clock_timestamp() \
                 ELSE chatroom_reads.updated_at \
             END \
         RETURNING id, user_id, chatroom_id, last_read_cursor, updated_at",
    )
    .bind(command.marker_id)
    .bind(command.user_id)
    .bind(command.chatroom_id)
    .bind(cursor)
    .fetch_one(connection)
    .await
    .map_err(|error| database_error("read_marker_upsert", error))?;

    Ok(ReadMarker {
        id: row.0,
        user_id: row.1,
        chatroom_id: row.2,
        last_read_cursor: row.3,
        updated_at: row.4,
    })
}

async fn cursor_for_existing_cursor(
    connection: &mut PgConnection,
    command: &MarkReadCommand,
    cursor: i64,
) -> Result<i64, ChatroomsRepositoryError> {
    let cursor = sqlx::query_scalar::<_, i64>(
        "SELECT cursor FROM conversation_events \
         WHERE conversation_id = $1 AND cursor = $2",
    )
    .bind(command.chatroom_id)
    .bind(cursor)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| database_error("read_cursor_validate", error))?;
    let Some(cursor) = cursor else {
        return Err(ChatroomsRepositoryError::CursorInvalid);
    };
    Ok(cursor)
}

async fn cursor_for_message_anchor(
    connection: &mut PgConnection,
    command: &MarkReadCommand,
    message_id: Uuid,
) -> Result<i64, ChatroomsRepositoryError> {
    let cursors = sqlx::query_scalar::<_, i64>(
        "SELECT event.cursor \
         FROM messages AS message \
         JOIN conversation_events AS event \
           ON event.conversation_id = message.chatroom_id \
          AND event.event_type = 'message.created' \
          AND event.event_version = 1 \
          AND event.payload ->> 'id' = message.id::TEXT \
         WHERE message.id = $1 AND message.chatroom_id = $2 \
         ORDER BY event.cursor \
         FOR SHARE OF message, event",
    )
    .bind(message_id)
    .bind(command.chatroom_id)
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| database_error("read_message_anchor", error))?;
    let [cursor] = cursors.as_slice() else {
        return Err(ChatroomsRepositoryError::CursorInvalid);
    };
    Ok(*cursor)
}
