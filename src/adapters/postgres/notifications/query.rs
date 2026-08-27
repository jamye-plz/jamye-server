use std::collections::BTreeMap;

use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::ports::push::{
    ListNotificationsQuery, NotificationPage, NotificationRecord, NotificationType,
    NotificationsRepositoryError,
};

use super::database_error;

type NotificationAccessRow = (
    bool,
    i64,
    Option<Uuid>,
    Option<String>,
    Option<Value>,
    Option<Uuid>,
    Option<Uuid>,
    Option<i64>,
    Option<OffsetDateTime>,
    Option<OffsetDateTime>,
);

pub(super) async fn list_notifications(
    pool: &PgPool,
    query: ListNotificationsQuery,
) -> Result<NotificationPage, NotificationsRepositoryError> {
    let fetch_limit = i64::from(query.limit) + 1;
    let rows = sqlx::query_as::<_, NotificationAccessRow>(
        "WITH cursor_row AS ( \
             SELECT created_at, id FROM notifications \
             WHERE id = $2 AND user_id = $1 \
         ), state AS ( \
             SELECT \
                 ($2::uuid IS NULL OR EXISTS (SELECT 1 FROM cursor_row)) AS cursor_valid, \
                 ( \
                     SELECT count(*) FROM notifications \
                     WHERE user_id = $1 AND read_at IS NULL \
                 ) AS unread_count \
         ), page AS ( \
             SELECT notification.id, notification.type, notification.payload, \
                    notification.topic_id, notification.conversation_id, \
                    notification.source_cursor, notification.read_at, \
                    notification.created_at \
             FROM notifications notification \
             CROSS JOIN state \
             WHERE notification.user_id = $1 AND state.cursor_valid \
               AND ( \
                   $2::uuid IS NULL \
                   OR (notification.created_at, notification.id) < ( \
                       SELECT created_at, id FROM cursor_row \
                   ) \
               ) \
             ORDER BY notification.created_at DESC, notification.id DESC \
             LIMIT $3 \
         ) \
         SELECT state.cursor_valid, state.unread_count, page.id, page.type, page.payload, \
                page.topic_id, page.conversation_id, page.source_cursor, page.read_at, \
                page.created_at \
         FROM state LEFT JOIN page ON TRUE \
         ORDER BY page.created_at DESC, page.id DESC",
    )
    .bind(query.user_id)
    .bind(query.after)
    .bind(fetch_limit)
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("notification_list", error))?;

    let first = rows
        .first()
        .ok_or(NotificationsRepositoryError::Unavailable)?;
    if !first.0 {
        return Err(NotificationsRepositoryError::CursorInvalid);
    }
    let unread_count =
        u64::try_from(first.1).map_err(|_| NotificationsRepositoryError::InvalidData)?;
    let mut items = rows
        .into_iter()
        .filter(|row| row.2.is_some())
        .map(notification_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    let has_more = items.len() > query.limit as usize;
    if has_more {
        items.truncate(query.limit as usize);
    }
    let next_cursor = has_more
        .then(|| items.last().map(|notification| notification.id.to_string()))
        .flatten();
    Ok(NotificationPage {
        items,
        next_cursor,
        unread_count,
    })
}

fn notification_from_row(
    row: NotificationAccessRow,
) -> Result<NotificationRecord, NotificationsRepositoryError> {
    let id = row.2.ok_or(NotificationsRepositoryError::InvalidData)?;
    let notification_type = NotificationType::parse(
        row.3
            .as_deref()
            .ok_or(NotificationsRepositoryError::InvalidData)?,
    )
    .ok_or(NotificationsRepositoryError::InvalidData)?;
    let args = notification_args(row.4.ok_or(NotificationsRepositoryError::InvalidData)?)?;
    if row.7.is_some_and(|cursor| cursor <= 0) {
        return Err(NotificationsRepositoryError::InvalidData);
    }
    Ok(NotificationRecord {
        id,
        notification_type,
        args,
        topic_id: row.5,
        conversation_id: row.6,
        source_cursor: row.7,
        read_at: row.8,
        created_at: row.9.ok_or(NotificationsRepositoryError::InvalidData)?,
    })
}

fn notification_args(
    payload: Value,
) -> Result<BTreeMap<String, Value>, NotificationsRepositoryError> {
    let object = payload
        .as_object()
        .ok_or(NotificationsRepositoryError::InvalidData)?;
    if object.len() > 16
        || object
            .iter()
            .any(|(key, value)| !valid_arg_key(key) || !scalar(value))
    {
        return Err(NotificationsRepositoryError::InvalidData);
    }
    Ok(object
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect())
}

fn valid_arg_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z'))
        && key.len() <= 64
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn scalar(value: &Value) -> bool {
    matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}
