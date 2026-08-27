use std::collections::BTreeMap;

use serde_json::{Map, Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::ports::push::{
    ClearTopicNotificationsCommand, NotificationClearReport, NotificationFanoutReport,
    NotificationsRepositoryError, RecordMessageNotificationCommand, RecordTopicNotificationCommand,
};

use super::database_error;

type InstallationRow = (Uuid, Uuid, i64, bool);

struct Fanout<'a> {
    topic_id: Uuid,
    conversation_id: Uuid,
    source_event_id: Uuid,
    source_message_id: Option<Uuid>,
    source_cursor: i64,
    notification_type: &'static str,
    display_name_key: &'static str,
    display_name: &'a str,
}

pub(super) async fn record_topic_created(
    connection: &mut PgConnection,
    command: &RecordTopicNotificationCommand,
) -> Result<NotificationFanoutReport, NotificationsRepositoryError> {
    validate_display_name(&command.author_display_name)?;
    let recipients = lock_live_recipients(connection, command.group_id, command.author_id).await?;
    let source_cursor = topic_source_cursor(connection, command).await?;
    fan_out(
        connection,
        &recipients,
        &Fanout {
            topic_id: command.topic_id,
            conversation_id: command.conversation_id,
            source_event_id: command.source_event_id,
            source_message_id: None,
            source_cursor,
            notification_type: "new_topic",
            display_name_key: "author_display_name",
            display_name: &command.author_display_name,
        },
    )
    .await
}

pub(super) async fn record_message_created(
    connection: &mut PgConnection,
    command: &RecordMessageNotificationCommand,
) -> Result<NotificationFanoutReport, NotificationsRepositoryError> {
    validate_display_name(&command.sender_display_name)?;
    let recipients = lock_live_recipients(connection, command.group_id, command.sender_id).await?;
    let source_cursor = message_source_cursor(connection, command).await?;
    fan_out(
        connection,
        &recipients,
        &Fanout {
            topic_id: command.topic_id,
            conversation_id: command.conversation_id,
            source_event_id: command.source_event_id,
            source_message_id: Some(command.source_message_id),
            source_cursor,
            notification_type: "chat_unread",
            display_name_key: "sender_display_name",
            display_name: &command.sender_display_name,
        },
    )
    .await
}

pub(super) async fn clear_topic_notifications(
    connection: &mut PgConnection,
    command: &ClearTopicNotificationsCommand,
) -> Result<NotificationClearReport, NotificationsRepositoryError> {
    let target = sqlx::query_as::<_, (Uuid, Uuid)>(
        "SELECT group_id, topic_id FROM chatrooms \
         WHERE id = $1 AND type = 'topic'",
    )
    .bind(command.conversation_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| database_error("notification_clear_target", error))?
    .ok_or(NotificationsRepositoryError::InvalidData)?;

    lock_live_group(connection, target.0).await?;
    lock_member(connection, target.0, command.user_id).await?;
    let read_cursor = sqlx::query_scalar::<_, i64>(
        "SELECT last_read_cursor FROM chatroom_reads \
         WHERE user_id = $1 AND chatroom_id = $2 \
         FOR SHARE",
    )
    .bind(command.user_id)
    .bind(command.conversation_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| database_error("notification_clear_marker", error))?
    .ok_or(NotificationsRepositoryError::InvalidData)?;

    let result = sqlx::query(
        "UPDATE notifications \
         SET read_at = clock_timestamp() \
         WHERE user_id = $1 AND topic_id = $2 AND conversation_id = $3 \
           AND type IN ('new_topic', 'chat_unread') \
           AND source_cursor <= $4 AND read_at IS NULL",
    )
    .bind(command.user_id)
    .bind(target.1)
    .bind(command.conversation_id)
    .bind(read_cursor)
    .execute(connection)
    .await
    .map_err(|error| database_error("notification_clear_bounded", error))?;
    Ok(NotificationClearReport {
        cleared_count: result.rows_affected(),
    })
}

async fn lock_live_recipients(
    connection: &mut PgConnection,
    group_id: Uuid,
    actor_id: Uuid,
) -> Result<Vec<Uuid>, NotificationsRepositoryError> {
    lock_live_group(connection, group_id).await?;
    let members = sqlx::query_scalar::<_, Uuid>(
        "SELECT user_id FROM memberships \
         WHERE group_id = $1 \
         ORDER BY user_id \
         FOR SHARE",
    )
    .bind(group_id)
    .fetch_all(connection)
    .await
    .map_err(|error| database_error("notification_membership_lock", error))?;
    if !members.contains(&actor_id) {
        return Err(NotificationsRepositoryError::InvalidData);
    }
    Ok(members
        .into_iter()
        .filter(|user_id| *user_id != actor_id)
        .collect())
}

async fn lock_live_group(
    connection: &mut PgConnection,
    group_id: Uuid,
) -> Result<(), NotificationsRepositoryError> {
    let live = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM groups \
         WHERE id = $1 AND deleted_at IS NULL \
         FOR SHARE",
    )
    .bind(group_id)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("notification_group_lock", error))?;
    if live.is_none() {
        return Err(NotificationsRepositoryError::InvalidData);
    }
    Ok(())
}

async fn lock_member(
    connection: &mut PgConnection,
    group_id: Uuid,
    user_id: Uuid,
) -> Result<(), NotificationsRepositoryError> {
    let membership = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM memberships \
         WHERE group_id = $1 AND user_id = $2 \
         FOR SHARE",
    )
    .bind(group_id)
    .bind(user_id)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("notification_member_lock", error))?;
    if membership.is_none() {
        return Err(NotificationsRepositoryError::InvalidData);
    }
    Ok(())
}

async fn topic_source_cursor(
    connection: &mut PgConnection,
    command: &RecordTopicNotificationCommand,
) -> Result<i64, NotificationsRepositoryError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT event.cursor \
         FROM topics topic \
         JOIN chatrooms conversation \
           ON conversation.group_id = topic.group_id \
          AND conversation.topic_id = topic.id \
          AND conversation.type = 'topic' \
         JOIN conversation_events event \
           ON event.id = $4 \
          AND event.conversation_id = conversation.id \
          AND event.event_type = 'topic.created' \
          AND event.event_version = 1 \
         WHERE topic.group_id = $1 AND topic.id = $2 AND conversation.id = $3 \
           AND event.payload ->> 'topic_id' = $2::uuid::text",
    )
    .bind(command.group_id)
    .bind(command.topic_id)
    .bind(command.conversation_id)
    .bind(command.source_event_id)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("notification_topic_source", error))?
    .ok_or(NotificationsRepositoryError::InvalidData)
}

async fn message_source_cursor(
    connection: &mut PgConnection,
    command: &RecordMessageNotificationCommand,
) -> Result<i64, NotificationsRepositoryError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT event.cursor \
         FROM topics topic \
         JOIN chatrooms conversation \
           ON conversation.group_id = topic.group_id \
          AND conversation.topic_id = topic.id \
          AND conversation.type = 'topic' \
         JOIN messages message \
           ON message.id = $5 \
          AND message.chatroom_id = conversation.id \
          AND message.sender_id = $6 \
          AND message.type = 'user' \
         JOIN conversation_events event \
           ON event.id = $4 \
          AND event.conversation_id = conversation.id \
          AND event.event_type = 'message.created' \
          AND event.event_version = 1 \
         WHERE topic.group_id = $1 AND topic.id = $2 AND conversation.id = $3 \
           AND event.payload ->> 'id' = $5::uuid::text",
    )
    .bind(command.group_id)
    .bind(command.topic_id)
    .bind(command.conversation_id)
    .bind(command.source_event_id)
    .bind(command.source_message_id)
    .bind(command.sender_id)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("notification_message_source", error))?
    .ok_or(NotificationsRepositoryError::InvalidData)
}

async fn fan_out(
    connection: &mut PgConnection,
    recipient_user_ids: &[Uuid],
    fanout: &Fanout<'_>,
) -> Result<NotificationFanoutReport, NotificationsRepositoryError> {
    let notification_count = u64::try_from(recipient_user_ids.len())
        .map_err(|_| NotificationsRepositoryError::InvalidData)?;
    if recipient_user_ids.is_empty() {
        return Ok(NotificationFanoutReport {
            notification_count,
            occurrence_count: 0,
        });
    }

    let notification_ids = upsert_notifications(connection, recipient_user_ids, fanout).await?;
    let installations = lock_installations(connection, recipient_user_ids).await?;
    let occurrence_count =
        insert_occurrences(connection, &notification_ids, installations, fanout).await?;

    Ok(NotificationFanoutReport {
        notification_count,
        occurrence_count,
    })
}

fn notification_args(fanout: &Fanout<'_>) -> Value {
    let mut args = Map::new();
    args.insert(
        fanout.display_name_key.to_owned(),
        Value::String(fanout.display_name.to_owned()),
    );
    Value::Object(args)
}

async fn upsert_notifications(
    connection: &mut PgConnection,
    recipient_user_ids: &[Uuid],
    fanout: &Fanout<'_>,
) -> Result<BTreeMap<Uuid, Uuid>, NotificationsRepositoryError> {
    let payload = notification_args(fanout);
    let dedup_key = format!("{}:{}", fanout.notification_type, fanout.topic_id);
    let mut notification_ids = BTreeMap::new();
    for user_id in recipient_user_ids {
        let notification_id =
            upsert_notification(connection, *user_id, fanout, &payload, &dedup_key).await?;
        notification_ids.insert(*user_id, notification_id);
    }
    Ok(notification_ids)
}

async fn insert_occurrences(
    connection: &mut PgConnection,
    notification_ids: &BTreeMap<Uuid, Uuid>,
    installations: Vec<InstallationRow>,
    fanout: &Fanout<'_>,
) -> Result<u64, NotificationsRepositoryError> {
    let mut occurrence_count = 0_u64;
    for installation in installations {
        if installation.2 <= 0 {
            return Err(NotificationsRepositoryError::InvalidData);
        }
        let notification_id = notification_ids
            .get(&installation.1)
            .copied()
            .ok_or(NotificationsRepositoryError::InvalidData)?;
        let inserted = insert_occurrence(connection, installation, notification_id, fanout).await?;
        occurrence_count = occurrence_count
            .checked_add(inserted)
            .ok_or(NotificationsRepositoryError::InvalidData)?;
    }
    Ok(occurrence_count)
}

async fn insert_occurrence(
    connection: &mut PgConnection,
    installation: InstallationRow,
    notification_id: Uuid,
    fanout: &Fanout<'_>,
) -> Result<u64, NotificationsRepositoryError> {
    let delivery_payload = json!({
        "type": fanout.notification_type,
        "notification_id": notification_id,
        "conversation_id": fanout.conversation_id,
        "message_id": fanout.source_message_id,
    });
    let inserted = sqlx::query(
        "INSERT INTO push_delivery_intents \
             (id, notification_id, source_event_id, source_message_id, \
              recipient_user_id, push_installation_id, installation_owner_epoch, \
              message_preview_enabled_snapshot, payload) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
         ON CONFLICT ON CONSTRAINT uq_push_delivery_source_installation DO NOTHING",
    )
    .bind(Uuid::new_v4())
    .bind(notification_id)
    .bind(fanout.source_event_id)
    .bind(fanout.source_message_id)
    .bind(installation.1)
    .bind(installation.0)
    .bind(installation.2)
    .bind(installation.3)
    .bind(delivery_payload)
    .execute(connection)
    .await
    .map_err(|error| database_error("notification_occurrence_insert", error))?;
    Ok(inserted.rows_affected())
}

async fn upsert_notification(
    connection: &mut PgConnection,
    user_id: Uuid,
    fanout: &Fanout<'_>,
    payload: &Value,
    dedup_key: &str,
) -> Result<Uuid, NotificationsRepositoryError> {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO notifications \
             (id, user_id, topic_id, conversation_id, source_cursor, type, payload, dedup_key) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         ON CONFLICT (user_id, dedup_key) WHERE dedup_key IS NOT NULL \
         DO UPDATE SET \
             source_cursor = GREATEST(notifications.source_cursor, EXCLUDED.source_cursor), \
             payload = CASE \
                 WHEN EXCLUDED.source_cursor > notifications.source_cursor \
                 THEN EXCLUDED.payload ELSE notifications.payload \
             END, \
             read_at = CASE \
                 WHEN EXCLUDED.source_cursor > notifications.source_cursor \
                 THEN NULL ELSE notifications.read_at \
             END, \
             created_at = CASE \
                 WHEN EXCLUDED.source_cursor > notifications.source_cursor \
                 THEN clock_timestamp() ELSE notifications.created_at \
             END \
         WHERE notifications.type = EXCLUDED.type \
           AND notifications.topic_id = EXCLUDED.topic_id \
           AND notifications.conversation_id = EXCLUDED.conversation_id \
         RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(fanout.topic_id)
    .bind(fanout.conversation_id)
    .bind(fanout.source_cursor)
    .bind(fanout.notification_type)
    .bind(payload)
    .bind(dedup_key)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("notification_upsert", error))?
    .ok_or(NotificationsRepositoryError::InvalidData)
}

async fn lock_installations(
    connection: &mut PgConnection,
    recipient_user_ids: &[Uuid],
) -> Result<Vec<InstallationRow>, NotificationsRepositoryError> {
    sqlx::query_as::<_, InstallationRow>(
        "SELECT id, user_id, owner_epoch, message_preview_enabled \
         FROM push_installations \
         WHERE user_id = ANY($1::UUID[]) AND disabled_at IS NULL \
         ORDER BY user_id, id \
         FOR SHARE",
    )
    .bind(recipient_user_ids.to_vec())
    .fetch_all(connection)
    .await
    .map_err(|error| database_error("notification_installation_lock", error))
}

fn validate_display_name(value: &str) -> Result<(), NotificationsRepositoryError> {
    if value.is_empty() || value.chars().count() > 64 || value.chars().any(char::is_control) {
        return Err(NotificationsRepositoryError::InvalidData);
    }
    Ok(())
}
