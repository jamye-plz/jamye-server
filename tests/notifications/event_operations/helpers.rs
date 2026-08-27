use std::sync::Arc;

use jamye_server::{
    adapters::postgres::{
        notifications::PostgresNotificationsRepository, transactions::SqlxTransactionManager,
    },
    application::notifications::NotificationTransactionOperations,
    ports::{
        push::{
            ClearTopicNotificationsCommand, NotificationClearReport, NotificationFanoutReport,
            RecordMessageNotificationCommand, RecordTopicNotificationCommand,
        },
        transactions::TransactionManager,
    },
};
use serde_json::json;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::TestResult;

pub(super) fn operations(pool: PgPool) -> NotificationTransactionOperations {
    NotificationTransactionOperations::new(Arc::new(PostgresNotificationsRepository::new(pool)))
}

pub(super) async fn committed_topic(
    operations: &NotificationTransactionOperations,
    transactions: &SqlxTransactionManager,
    command: RecordTopicNotificationCommand,
) -> TestResult<NotificationFanoutReport> {
    let mut transaction = transactions.begin().await?;
    let report = operations
        .record_topic_created(transaction.as_mut(), &command)
        .await?;
    transactions.commit(transaction).await?;
    Ok(report)
}

pub(super) async fn committed_message(
    operations: &NotificationTransactionOperations,
    transactions: &SqlxTransactionManager,
    command: RecordMessageNotificationCommand,
) -> TestResult<NotificationFanoutReport> {
    let mut transaction = transactions.begin().await?;
    let report = operations
        .record_message_created(transaction.as_mut(), &command)
        .await?;
    transactions.commit(transaction).await?;
    Ok(report)
}

pub(super) async fn committed_clear(
    operations: &NotificationTransactionOperations,
    transactions: &SqlxTransactionManager,
    command: ClearTopicNotificationsCommand,
) -> TestResult<NotificationClearReport> {
    let mut transaction = transactions.begin().await?;
    let report = operations
        .clear_topic_notifications(transaction.as_mut(), &command)
        .await?;
    transactions.commit(transaction).await?;
    Ok(report)
}

#[derive(Clone, Copy)]
pub(super) struct SourceEvent {
    pub(super) event_id: Uuid,
    pub(super) message_id: Uuid,
    pub(super) cursor: i64,
}

pub(super) fn message_command(
    topology: &Topology,
    source: SourceEvent,
) -> RecordMessageNotificationCommand {
    RecordMessageNotificationCommand {
        group_id: topology.group_id,
        topic_id: topology.topic_id,
        conversation_id: topology.conversation_id,
        source_event_id: source.event_id,
        source_message_id: source.message_id,
        sender_id: topology.owner_id,
        sender_display_name: "메시지 작성자".to_owned(),
    }
}

pub(super) async fn insert_message_event(
    pool: &PgPool,
    topology: &Topology,
    body: &str,
) -> TestResult<SourceEvent> {
    let message_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO messages \
             (id, chatroom_id, sender_id, client_msg_id, body, type) \
         VALUES ($1, $2, $3, $4, $5, 'user')",
    )
    .bind(message_id)
    .bind(topology.conversation_id)
    .bind(topology.owner_id)
    .bind(Uuid::new_v4())
    .bind(body)
    .execute(pool)
    .await?;
    let event_id = Uuid::new_v4();
    let cursor = sqlx::query_scalar::<_, i64>(
        "INSERT INTO conversation_events \
             (id, conversation_id, event_type, event_version, payload) \
         VALUES ($1, $2, 'message.created', 1, $3) \
         RETURNING cursor",
    )
    .bind(event_id)
    .bind(topology.conversation_id)
    .bind(json!({
        "id": message_id,
        "chatroom_id": topology.conversation_id,
        "sender_id": topology.owner_id,
        "body": body,
    }))
    .fetch_one(pool)
    .await?;
    Ok(SourceEvent {
        event_id,
        message_id,
        cursor,
    })
}

pub(super) async fn insert_topic_event(
    pool: &PgPool,
    topology: &Topology,
) -> TestResult<SourceEvent> {
    let event_id = Uuid::new_v4();
    let cursor = sqlx::query_scalar::<_, i64>(
        "INSERT INTO conversation_events \
             (id, conversation_id, event_type, event_version, payload) \
         VALUES ($1, $2, 'topic.created', 1, $3) \
         RETURNING cursor",
    )
    .bind(event_id)
    .bind(topology.conversation_id)
    .bind(json!({"topic_id": topology.topic_id}))
    .fetch_one(pool)
    .await?;
    Ok(SourceEvent {
        event_id,
        message_id: Uuid::nil(),
        cursor,
    })
}

pub(super) async fn chat_notification(
    pool: &PgPool,
    user_id: Uuid,
) -> TestResult<(Uuid, i64, Option<OffsetDateTime>)> {
    Ok(sqlx::query_as(
        "SELECT id, source_cursor, read_at FROM notifications \
         WHERE user_id = $1 AND type = 'chat_unread'",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?)
}

pub(super) async fn notification_id(
    pool: &PgPool,
    user_id: Uuid,
    topic_id: Uuid,
    notification_type: &str,
) -> TestResult<Uuid> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM notifications WHERE user_id = $1 AND topic_id = $2 AND type = $3",
    )
    .bind(user_id)
    .bind(topic_id)
    .bind(notification_type)
    .fetch_one(pool)
    .await?)
}

pub(super) async fn notification_count(pool: &PgPool, user_id: Uuid) -> TestResult<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM notifications WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(pool)
            .await?,
    )
}

pub(super) async fn occurrence_count(pool: &PgPool, user_id: Uuid) -> TestResult<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM push_delivery_intents WHERE recipient_user_id = $1",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?)
}

pub(super) async fn insert_direct_notification(
    pool: &PgPool,
    user_id: Uuid,
    topic_id: Uuid,
    conversation_id: Uuid,
    source_cursor: i64,
    suffix: &str,
) -> TestResult<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO notifications \
             (id, user_id, topic_id, conversation_id, source_cursor, type, payload, dedup_key) \
         VALUES ($1, $2, $3, $4, $5, 'chat_unread', '{}', $6)",
    )
    .bind(id)
    .bind(user_id)
    .bind(topic_id)
    .bind(conversation_id)
    .bind(source_cursor)
    .bind(format!("chat_unread:{topic_id}:{suffix}"))
    .execute(pool)
    .await?;
    Ok(id)
}

pub(super) async fn is_read(pool: &PgPool, notification_id: Uuid) -> TestResult<bool> {
    Ok(
        sqlx::query_scalar("SELECT read_at IS NOT NULL FROM notifications WHERE id = $1")
            .bind(notification_id)
            .fetch_one(pool)
            .await?,
    )
}

pub(super) struct Topology {
    pub(super) owner_id: Uuid,
    pub(super) recipient_id: Uuid,
    pub(super) no_install_id: Uuid,
    pub(super) outsider_id: Uuid,
    pub(super) group_id: Uuid,
    pub(super) topic_id: Uuid,
    pub(super) conversation_id: Uuid,
    pub(super) other_topic_id: Uuid,
    pub(super) other_conversation_id: Uuid,
}

impl Topology {
    pub(super) async fn new(pool: &PgPool) -> TestResult<Self> {
        let owner_id = insert_user(pool, "작성자").await?;
        let recipient_id = insert_user(pool, "설치 보유 멤버").await?;
        let no_install_id = insert_user(pool, "설치 없는 멤버").await?;
        let outsider_id = insert_user(pool, "외부 사용자").await?;
        let group_id = Uuid::new_v4();
        sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, '알림 그룹', $2)")
            .bind(group_id)
            .bind(owner_id)
            .execute(pool)
            .await?;
        for (user_id, role) in [
            (owner_id, "owner"),
            (recipient_id, "member"),
            (no_install_id, "member"),
        ] {
            sqlx::query(
                "INSERT INTO memberships (id, group_id, user_id, role) \
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(Uuid::new_v4())
            .bind(group_id)
            .bind(user_id)
            .bind(role)
            .execute(pool)
            .await?;
        }
        sqlx::query(
            "INSERT INTO chatrooms (id, group_id, type, topic_id) \
             VALUES ($1, $2, 'main', NULL)",
        )
        .bind(Uuid::new_v4())
        .bind(group_id)
        .execute(pool)
        .await?;

        let topic_id = Uuid::new_v4();
        let other_topic_id = Uuid::new_v4();
        for (topic_id, fingerprint, title) in [
            (topic_id, "a".repeat(64), "첫 주제"),
            (other_topic_id, "b".repeat(64), "다른 주제"),
        ] {
            sqlx::query(
                "INSERT INTO topics \
                     (id, group_id, author_id, idempotency_key, request_fingerprint, title) \
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(topic_id)
            .bind(group_id)
            .bind(owner_id)
            .bind(Uuid::new_v4())
            .bind(fingerprint)
            .bind(title)
            .execute(pool)
            .await?;
        }
        let conversation_id = Uuid::new_v4();
        let other_conversation_id = Uuid::new_v4();
        for (conversation_id, topic_id) in [
            (conversation_id, topic_id),
            (other_conversation_id, other_topic_id),
        ] {
            sqlx::query(
                "INSERT INTO chatrooms (id, group_id, type, topic_id) \
                 VALUES ($1, $2, 'topic', $3)",
            )
            .bind(conversation_id)
            .bind(group_id)
            .bind(topic_id)
            .execute(pool)
            .await?;
        }
        insert_installation(pool, recipient_id, "member-installation", true).await?;
        insert_installation(pool, outsider_id, "outsider-installation", true).await?;

        Ok(Self {
            owner_id,
            recipient_id,
            no_install_id,
            outsider_id,
            group_id,
            topic_id,
            conversation_id,
            other_topic_id,
            other_conversation_id,
        })
    }
}

async fn insert_user(pool: &PgPool, nickname: &str) -> TestResult<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, $2)")
        .bind(id)
        .bind(nickname)
        .execute(pool)
        .await?;
    Ok(id)
}

async fn insert_installation(
    pool: &PgPool,
    user_id: Uuid,
    installation_id: &str,
    message_preview_enabled: bool,
) -> TestResult {
    sqlx::query(
        "INSERT INTO push_installations \
             (id, user_id, installation_id, platform, provider, token, environment, \
              message_preview_enabled) \
         VALUES ($1, $2, $3, 'ios', 'expo', $4, 'development', $5)",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(installation_id)
    .bind(format!("ExponentPushToken[{installation_id}]"))
    .bind(message_preview_enabled)
    .execute(pool)
    .await?;
    Ok(())
}
