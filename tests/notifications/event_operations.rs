use std::collections::BTreeSet;

use jamye_server::{
    adapters::postgres::transactions::SqlxTransactionManager,
    ports::push::{
        ClearTopicNotificationsCommand, NotificationClearReport, NotificationFanoutReport,
        RecordTopicNotificationCommand,
    },
};
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

#[path = "event_operations/helpers.rs"]
mod helpers;

use helpers::{
    Topology, chat_notification, committed_clear, committed_message, committed_topic,
    conversation_chat_notification, insert_direct_notification, insert_message_event,
    insert_message_event_in, insert_topic_event, is_read, main_message_command, message_command,
    notification_count, notification_id, notification_payload, occurrence_count, operations,
    push_delivery_payload,
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
async fn notification_args_cover_context_shapes_and_do_not_change_push_payloads() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let topology = Topology::new(&pool).await?;
    let operations = operations(pool.clone());
    let transactions = SqlxTransactionManager::new(pool.clone());

    let topic_event = insert_topic_event(&pool, &topology).await?;
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
    .await?;
    let new_topic_id =
        notification_id(&pool, topology.recipient_id, topology.topic_id, "new_topic").await?;
    assert_eq!(
        notification_payload(
            &pool,
            topology.recipient_id,
            topology.conversation_id,
            "new_topic",
        )
        .await?,
        json!({
            "author_display_name": "주제 작성자",
            "group_name": "알림 그룹",
            "topic_title": "첫 주제",
        })
    );
    assert_eq!(
        push_delivery_payload(&pool, topology.recipient_id, topic_event.event_id).await?,
        json!({
            "type": "new_topic",
            "notification_id": new_topic_id,
            "conversation_id": topology.conversation_id,
            "message_id": Value::Null,
        })
    );

    let topic_message = insert_message_event(&pool, &topology, "topic message").await?;
    committed_message(
        &operations,
        &transactions,
        message_command(&topology, topic_message),
    )
    .await?;
    let (topic_chat_id, _, _, _) =
        conversation_chat_notification(&pool, topology.recipient_id, topology.conversation_id)
            .await?;
    let first_topic_chat_args = json!({
        "sender_display_name": "메시지 작성자",
        "group_name": "알림 그룹",
        "topic_title": "첫 주제",
    });
    assert_eq!(
        notification_payload(
            &pool,
            topology.recipient_id,
            topology.conversation_id,
            "chat_unread",
        )
        .await?,
        first_topic_chat_args
    );
    assert_eq!(
        push_delivery_payload(&pool, topology.recipient_id, topic_message.event_id).await?,
        json!({
            "type": "chat_unread",
            "notification_id": topic_chat_id,
            "conversation_id": topology.conversation_id,
            "message_id": topic_message.message_id,
        })
    );

    sqlx::query("UPDATE groups SET name = '알림 그룹 변경' WHERE id = $1")
        .bind(topology.group_id)
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE topics SET title = '첫 주제 변경' WHERE id = $1")
        .bind(topology.topic_id)
        .execute(&pool)
        .await?;
    committed_message(
        &operations,
        &transactions,
        message_command(&topology, topic_message),
    )
    .await?;
    assert_eq!(
        notification_payload(
            &pool,
            topology.recipient_id,
            topology.conversation_id,
            "chat_unread",
        )
        .await?,
        first_topic_chat_args
    );

    let newer_topic_message = insert_message_event(&pool, &topology, "newer topic message").await?;
    committed_message(
        &operations,
        &transactions,
        message_command(&topology, newer_topic_message),
    )
    .await?;
    assert_eq!(
        notification_payload(
            &pool,
            topology.recipient_id,
            topology.conversation_id,
            "chat_unread",
        )
        .await?,
        json!({
            "sender_display_name": "메시지 작성자",
            "group_name": "알림 그룹 변경",
            "topic_title": "첫 주제 변경",
        })
    );

    let main_message = insert_message_event_in(
        &pool,
        &topology,
        topology.main_conversation_id,
        "main message",
    )
    .await?;
    committed_message(
        &operations,
        &transactions,
        main_message_command(&topology, main_message),
    )
    .await?;
    let (main_chat_id, _, _, _) =
        conversation_chat_notification(&pool, topology.recipient_id, topology.main_conversation_id)
            .await?;
    assert_eq!(
        notification_payload(
            &pool,
            topology.recipient_id,
            topology.main_conversation_id,
            "chat_unread",
        )
        .await?,
        json!({
            "sender_display_name": "메시지 작성자",
            "group_name": "알림 그룹 변경",
        })
    );
    assert_eq!(
        push_delivery_payload(&pool, topology.recipient_id, main_message.event_id).await?,
        json!({
            "type": "chat_unread",
            "notification_id": main_chat_id,
            "conversation_id": topology.main_conversation_id,
            "message_id": main_message.message_id,
        })
    );

    let legacy_notification = insert_direct_notification(
        &pool,
        topology.recipient_id,
        topology.other_topic_id,
        topology.other_conversation_id,
        topic_event.cursor,
        "legacy-empty-args",
    )
    .await?;
    let legacy_payload: Value =
        sqlx::query_scalar("SELECT payload FROM notifications WHERE id = $1")
            .bind(legacy_notification)
            .fetch_one(&pool)
            .await?;
    assert_eq!(legacy_payload, json!({}));

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

#[tokio::test]
async fn main_chatroom_messages_notify_per_conversation_and_clear_through_the_marker() -> TestResult
{
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let topology = Topology::new(&pool).await?;
    let operations = operations(pool.clone());
    let transactions = SqlxTransactionManager::new(pool.clone());
    let main_id = topology.main_conversation_id;

    // First main-chatroom message: one chat_unread row per live member
    // (owner excluded), one occurrence for the member with an installation.
    let first = insert_message_event_in(&pool, &topology, main_id, "main-first-body").await?;
    assert_eq!(
        committed_message(
            &operations,
            &transactions,
            main_message_command(&topology, first),
        )
        .await?,
        NotificationFanoutReport {
            notification_count: 2,
            occurrence_count: 1,
        }
    );
    let (main_notification_id, main_topic_id, main_cursor, main_read_at) =
        conversation_chat_notification(&pool, topology.recipient_id, main_id).await?;
    assert_eq!(main_topic_id, None);
    assert_eq!(main_cursor, first.cursor);
    assert_eq!(main_read_at, None);

    // A second main message coalesces into the same row and advances it.
    let second = insert_message_event_in(&pool, &topology, main_id, "main-second-body").await?;
    assert_eq!(
        committed_message(
            &operations,
            &transactions,
            main_message_command(&topology, second),
        )
        .await?,
        NotificationFanoutReport {
            notification_count: 2,
            occurrence_count: 1,
        }
    );
    let (coalesced_id, _, coalesced_cursor, _) =
        conversation_chat_notification(&pool, topology.recipient_id, main_id).await?;
    assert_eq!(coalesced_id, main_notification_id);
    assert_eq!(coalesced_cursor, second.cursor);
    assert_eq!(notification_count(&pool, topology.recipient_id).await?, 1);
    assert_eq!(occurrence_count(&pool, topology.recipient_id).await?, 2);

    // A topic message keeps its own row: main and topic keys never collide.
    let topic_message = insert_message_event(&pool, &topology, "topic-body").await?;
    committed_message(
        &operations,
        &transactions,
        message_command(&topology, topic_message),
    )
    .await?;
    let topic_notification_id = notification_id(
        &pool,
        topology.recipient_id,
        topology.topic_id,
        "chat_unread",
    )
    .await?;
    assert_ne!(topic_notification_id, main_notification_id);
    assert_eq!(notification_count(&pool, topology.recipient_id).await?, 2);

    // Reading the main chatroom up to the second cursor clears only its row.
    sqlx::query(
        "INSERT INTO chatroom_reads (id, user_id, chatroom_id, last_read_cursor) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(topology.recipient_id)
    .bind(main_id)
    .bind(second.cursor)
    .execute(&pool)
    .await?;
    assert_eq!(
        committed_clear(
            &operations,
            &transactions,
            ClearTopicNotificationsCommand {
                user_id: topology.recipient_id,
                conversation_id: main_id,
            },
        )
        .await?,
        NotificationClearReport { cleared_count: 1 }
    );
    assert!(is_read(&pool, main_notification_id).await?);
    assert!(!is_read(&pool, topic_notification_id).await?);

    // A later main message reopens the same row as unread.
    let third = insert_message_event_in(&pool, &topology, main_id, "main-third-body").await?;
    committed_message(
        &operations,
        &transactions,
        main_message_command(&topology, third),
    )
    .await?;
    let (reopened_id, _, reopened_cursor, reopened_read_at) =
        conversation_chat_notification(&pool, topology.recipient_id, main_id).await?;
    assert_eq!(reopened_id, main_notification_id);
    assert_eq!(reopened_cursor, third.cursor);
    assert_eq!(reopened_read_at, None);
    Ok(())
}
