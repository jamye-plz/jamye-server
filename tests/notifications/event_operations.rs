use std::collections::BTreeSet;

use jamye_server::{
    adapters::postgres::transactions::SqlxTransactionManager,
    ports::push::{
        ClearTopicNotificationsCommand, NotificationClearReport, NotificationFanoutReport,
        RecordTopicNotificationCommand,
    },
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

#[path = "event_operations/helpers.rs"]
mod helpers;

use helpers::{
    Topology, chat_notification, committed_clear, committed_message, committed_topic,
    insert_direct_notification, insert_message_event, insert_topic_event, is_read, message_command,
    notification_count, notification_id, occurrence_count, operations,
};

#[tokio::test]
async fn distinct_messages_coalesce_history_but_keep_one_occurrence_per_source_event() -> TestResult
{
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let topology = Topology::new(&pool).await?;
    let operations = operations(pool.clone());
    let transactions = SqlxTransactionManager::new(pool.clone());

    let first = insert_message_event(&pool, &topology, "first-private-body").await?;
    assert_eq!(
        committed_message(
            &operations,
            &transactions,
            message_command(&topology, first),
        )
        .await?,
        NotificationFanoutReport {
            notification_count: 2,
            occurrence_count: 1,
        }
    );

    let (notification_id, first_cursor, _) =
        chat_notification(&pool, topology.recipient_id).await?;
    assert_eq!(first_cursor, first.cursor);
    let first_read_at = sqlx::query_scalar::<_, OffsetDateTime>(
        "UPDATE notifications SET read_at = clock_timestamp() \
         WHERE id = $1 RETURNING read_at",
    )
    .bind(notification_id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(
        committed_message(
            &operations,
            &transactions,
            message_command(&topology, first),
        )
        .await?,
        NotificationFanoutReport {
            notification_count: 2,
            occurrence_count: 0,
        }
    );
    assert_eq!(
        chat_notification(&pool, topology.recipient_id).await?,
        (notification_id, first.cursor, Some(first_read_at))
    );

    let second = insert_message_event(&pool, &topology, "second-private-body").await?;
    assert_eq!(
        committed_message(
            &operations,
            &transactions,
            message_command(&topology, second),
        )
        .await?,
        NotificationFanoutReport {
            notification_count: 2,
            occurrence_count: 1,
        }
    );
    assert_eq!(
        chat_notification(&pool, topology.recipient_id).await?,
        (notification_id, second.cursor, None)
    );

    assert_eq!(notification_count(&pool, topology.owner_id).await?, 0);
    assert_eq!(notification_count(&pool, topology.recipient_id).await?, 1);
    assert_eq!(notification_count(&pool, topology.no_install_id).await?, 1);
    assert_eq!(notification_count(&pool, topology.outsider_id).await?, 0);
    assert_eq!(occurrence_count(&pool, topology.no_install_id).await?, 0);
    assert_eq!(occurrence_count(&pool, topology.outsider_id).await?, 0);

    let occurrences = sqlx::query_as::<_, (Uuid, Uuid, Uuid, String, bool)>(
        "SELECT source_event_id, source_message_id, notification_id, status, \
                message_preview_enabled_snapshot \
         FROM push_delivery_intents WHERE recipient_user_id = $1",
    )
    .bind(topology.recipient_id)
    .fetch_all(&pool)
    .await?;
    assert_eq!(occurrences.len(), 2);
    assert_eq!(
        occurrences
            .iter()
            .map(|row| (row.0, row.1))
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            (first.event_id, first.message_id),
            (second.event_id, second.message_id),
        ])
    );
    for occurrence in occurrences {
        assert_eq!(occurrence.2, notification_id);
        assert_eq!(occurrence.3, "pending");
        assert!(occurrence.4);
    }

    let leaked: i64 = sqlx::query_scalar(
        "SELECT \
             (SELECT count(*) FROM notifications \
              WHERE payload::text LIKE '%private-body%') \
           + (SELECT count(*) FROM push_delivery_intents \
              WHERE payload::text LIKE '%private-body%')",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(leaked, 0);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn topic_read_clears_only_owner_topic_rows_through_the_canonical_marker() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let topology = Topology::new(&pool).await?;
    let operations = operations(pool.clone());
    let transactions = SqlxTransactionManager::new(pool.clone());

    let topic_event = insert_topic_event(&pool, &topology).await?;
    assert_eq!(
        committed_topic(
            &operations,
            &transactions,
            RecordTopicNotificationCommand {
                group_id: topology.group_id,
                topic_id: topology.topic_id,
                conversation_id: topology.conversation_id,
                source_event_id: topic_event.event_id,
                author_id: topology.owner_id,
                author_display_name: "주제 작성자".to_owned(),
            },
        )
        .await?,
        NotificationFanoutReport {
            notification_count: 2,
            occurrence_count: 1,
        }
    );
    let new_topic_id =
        notification_id(&pool, topology.recipient_id, topology.topic_id, "new_topic").await?;

    let message_event = insert_message_event(&pool, &topology, "bounded-private-body").await?;
    committed_message(
        &operations,
        &transactions,
        message_command(&topology, message_event),
    )
    .await?;
    let chat_unread_id = notification_id(
        &pool,
        topology.recipient_id,
        topology.topic_id,
        "chat_unread",
    )
    .await?;
    let other_topic_notification = insert_direct_notification(
        &pool,
        topology.recipient_id,
        topology.other_topic_id,
        topology.other_conversation_id,
        topic_event.cursor,
        "other-topic",
    )
    .await?;
    let foreign_notification = insert_direct_notification(
        &pool,
        topology.outsider_id,
        topology.topic_id,
        topology.conversation_id,
        topic_event.cursor,
        "foreign-user",
    )
    .await?;

    sqlx::query(
        "INSERT INTO chatroom_reads (id, user_id, chatroom_id, last_read_cursor) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(topology.recipient_id)
    .bind(topology.conversation_id)
    .bind(topic_event.cursor)
    .execute(&pool)
    .await?;
    assert_eq!(
        committed_clear(
            &operations,
            &transactions,
            ClearTopicNotificationsCommand {
                user_id: topology.recipient_id,
                conversation_id: topology.conversation_id,
            },
        )
        .await?,
        NotificationClearReport { cleared_count: 1 }
    );
    assert!(is_read(&pool, new_topic_id).await?);
    assert!(!is_read(&pool, chat_unread_id).await?);
    assert!(!is_read(&pool, other_topic_notification).await?);
    assert!(!is_read(&pool, foreign_notification).await?);

    sqlx::query(
        "UPDATE chatroom_reads SET last_read_cursor = $3, updated_at = clock_timestamp() \
         WHERE user_id = $1 AND chatroom_id = $2",
    )
    .bind(topology.recipient_id)
    .bind(topology.conversation_id)
    .bind(message_event.cursor)
    .execute(&pool)
    .await?;
    assert_eq!(
        committed_clear(
            &operations,
            &transactions,
            ClearTopicNotificationsCommand {
                user_id: topology.recipient_id,
                conversation_id: topology.conversation_id,
            },
        )
        .await?,
        NotificationClearReport { cleared_count: 1 }
    );
    assert!(is_read(&pool, chat_unread_id).await?);
    assert!(!is_read(&pool, other_topic_notification).await?);
    assert!(!is_read(&pool, foreign_notification).await?);
    assert_eq!(
        committed_clear(
            &operations,
            &transactions,
            ClearTopicNotificationsCommand {
                user_id: topology.recipient_id,
                conversation_id: topology.conversation_id,
            },
        )
        .await?,
        NotificationClearReport { cleared_count: 0 }
    );

    pool.close().await;
    database.dispose().await
}
