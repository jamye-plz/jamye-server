use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::ports::account_deletion::{ANONYMOUS_AUTHOR_NICKNAME, AccountDeletionRepositoryError};

use super::database_error;

type LockedPayloadRow = (Uuid, Value);

pub(super) async fn scrub_retained_payloads(
    connection: &mut PgConnection,
    user_id: Uuid,
    tombstone_user_id: Uuid,
) -> Result<(), AccountDeletionRepositoryError> {
    let original_user_id = user_id.to_string();
    let tombstone_user_id = tombstone_user_id.to_string();
    scrub_payload_rows(
        connection,
        &original_user_id,
        &tombstone_user_id,
        "account_deletion_conversation_payload_scrub",
        "SELECT id, payload FROM conversation_events \
         WHERE payload::text LIKE ('%' || $1 || '%') \
         ORDER BY id FOR UPDATE",
        "UPDATE conversation_events SET payload = $2 WHERE id = $1",
    )
    .await?;
    scrub_payload_rows(
        connection,
        &original_user_id,
        &tombstone_user_id,
        "account_deletion_outbox_payload_scrub",
        "SELECT id, payload FROM outbox_events \
         WHERE intent_type = 'conversation' \
           AND payload::text LIKE ('%' || $1 || '%') \
         ORDER BY id FOR UPDATE",
        "UPDATE outbox_events SET payload = $2 WHERE id = $1",
    )
    .await
}

async fn scrub_payload_rows(
    connection: &mut PgConnection,
    original_user_id: &str,
    tombstone_user_id: &str,
    operation: &'static str,
    select: &'static str,
    update: &'static str,
) -> Result<(), AccountDeletionRepositoryError> {
    let rows = sqlx::query_as::<_, LockedPayloadRow>(select)
        .bind(original_user_id)
        .fetch_all(&mut *connection)
        .await
        .map_err(|error| database_error(operation, error))?;
    for (row_id, mut payload) in rows {
        if !replace_exact_string(&mut payload, original_user_id, tombstone_user_id) {
            continue;
        }
        anonymize_profile_fields(&mut payload);
        let result = sqlx::query(update)
            .bind(row_id)
            .bind(payload)
            .execute(&mut *connection)
            .await
            .map_err(|error| database_error(operation, error))?;
        if result.rows_affected() != 1 {
            return Err(AccountDeletionRepositoryError::InvalidData);
        }
    }
    Ok(())
}

pub(super) async fn scrub_retained_notification_profiles(
    connection: &mut PgConnection,
    user_id: Uuid,
) -> Result<(), AccountDeletionRepositoryError> {
    let rows = sqlx::query_as::<_, LockedPayloadRow>(
        "SELECT notification.id, notification.payload \
         FROM notifications notification \
         JOIN conversation_events event \
           ON event.conversation_id = notification.conversation_id \
          AND event.cursor = notification.source_cursor \
         WHERE event.payload ->> 'sender_id' = $1::UUID::TEXT \
            OR event.payload ->> 'author_id' = $1::UUID::TEXT \
         ORDER BY notification.id \
         FOR UPDATE OF notification",
    )
    .bind(user_id)
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| database_error("account_deletion_notification_profile_lock", error))?;

    for (notification_id, mut payload) in rows {
        if !anonymize_profile_fields(&mut payload) {
            continue;
        }
        let result = sqlx::query("UPDATE notifications SET payload = $2 WHERE id = $1")
            .bind(notification_id)
            .bind(payload)
            .execute(&mut *connection)
            .await
            .map_err(|error| {
                database_error("account_deletion_notification_profile_scrub", error)
            })?;
        if result.rows_affected() != 1 {
            return Err(AccountDeletionRepositoryError::InvalidData);
        }
    }
    Ok(())
}

fn replace_exact_string(value: &mut Value, original: &str, replacement: &str) -> bool {
    match value {
        Value::String(current) if current == original => {
            *current = replacement.to_owned();
            true
        }
        Value::Array(values) => {
            let mut changed = false;
            for value in values {
                changed |= replace_exact_string(value, original, replacement);
            }
            changed
        }
        Value::Object(fields) => {
            let mut changed = false;
            for value in fields.values_mut() {
                changed |= replace_exact_string(value, original, replacement);
            }
            changed
        }
        _ => false,
    }
}

fn anonymize_profile_fields(value: &mut Value) -> bool {
    match value {
        Value::Array(values) => {
            let mut changed = false;
            for value in values {
                changed |= anonymize_profile_fields(value);
            }
            changed
        }
        Value::Object(fields) => {
            let mut changed = false;
            for (key, value) in fields {
                if is_nickname_key(key) {
                    let anonymous = Value::String(ANONYMOUS_AUTHOR_NICKNAME.to_owned());
                    if *value != anonymous {
                        *value = anonymous;
                        changed = true;
                    }
                } else if is_avatar_key(key) {
                    if !value.is_null() {
                        *value = Value::Null;
                        changed = true;
                    }
                } else {
                    changed |= anonymize_profile_fields(value);
                }
            }
            changed
        }
        _ => false,
    }
}

fn is_nickname_key(key: &str) -> bool {
    key == "nickname"
        || key == "display_name"
        || key.ends_with("_nickname")
        || key.ends_with("_display_name")
}

fn is_avatar_key(key: &str) -> bool {
    key == "avatar_url" || key.ends_with("_avatar_url")
}
