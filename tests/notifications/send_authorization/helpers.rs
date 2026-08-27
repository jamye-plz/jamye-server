use jamye_server::{
    adapters::postgres::{push::PostgresPushRepository, transactions::SqlxTransactionManager},
    ports::{
        push::{
            AuthorizedPushDelivery, DeletePushInstallationCommand, PushDeliveryClaim,
            PushInstallationRecord, PushRepository, PushSendAuthorizationRepository,
            UpdatePushInstallationCommand,
        },
        transactions::TransactionManager,
    },
};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::TestResult;

pub(crate) const MESSAGE_BODY: &str = "authorization-private-message-body";
const CLAIM_OWNER: &str = "task-9-authorization-worker";

pub(crate) async fn committed_authorize(
    repository: &PostgresPushRepository,
    transactions: &SqlxTransactionManager,
    claim: &PushDeliveryClaim,
) -> TestResult<Option<AuthorizedPushDelivery>> {
    let mut transaction = transactions.begin().await?;
    let result = repository.authorize_send(transaction.as_mut(), claim).await;
    match result {
        Ok(authorization) => {
            transactions.commit(transaction).await?;
            Ok(authorization)
        }
        Err(error) => {
            transactions.rollback(transaction).await?;
            Err(error.into())
        }
    }
}

pub(crate) async fn committed_update(
    repository: &PostgresPushRepository,
    transactions: &SqlxTransactionManager,
    command: UpdatePushInstallationCommand,
) -> TestResult<PushInstallationRecord> {
    let mut transaction = transactions.begin().await?;
    let result = repository
        .update_installation(transaction.as_mut(), &command)
        .await;
    match result {
        Ok(installation) => {
            transactions.commit(transaction).await?;
            Ok(installation)
        }
        Err(error) => {
            transactions.rollback(transaction).await?;
            Err(error.into())
        }
    }
}

pub(crate) async fn committed_delete(
    repository: &PostgresPushRepository,
    transactions: &SqlxTransactionManager,
    command: DeletePushInstallationCommand,
) -> TestResult {
    let mut transaction = transactions.begin().await?;
    let result = repository
        .delete_installation(transaction.as_mut(), &command)
        .await;
    match result {
        Ok(()) => {
            transactions.commit(transaction).await?;
            Ok(())
        }
        Err(error) => {
            transactions.rollback(transaction).await?;
            Err(error.into())
        }
    }
}

pub(crate) struct SendTopology {
    pub(crate) owner_id: Uuid,
    pub(crate) recipient_id: Uuid,
    pub(crate) group_id: Uuid,
    pub(crate) conversation_id: Uuid,
    pub(crate) message_id: Uuid,
    pub(crate) notification_id: Uuid,
    pub(crate) installation_id: Uuid,
    pub(crate) public_installation_id: String,
    pub(crate) expo_token: String,
    pub(crate) occurrence_id: Uuid,
    pub(crate) claim: PushDeliveryClaim,
}

impl SendTopology {
    pub(crate) async fn new(pool: &PgPool) -> TestResult<Self> {
        let owner_id = insert_user(pool, "authorization sender").await?;
        let recipient_id = insert_user(pool, "authorization recipient").await?;
        let group_id = insert_group(pool, owner_id, recipient_id).await?;
        let (topic_id, conversation_id) = insert_topic(pool, group_id, owner_id).await?;
        let (message_id, source_event_id, source_cursor) =
            insert_message_event(pool, conversation_id, owner_id).await?;
        let notification_id =
            insert_notification(pool, recipient_id, topic_id, conversation_id, source_cursor)
                .await?;
        let public_installation_id = format!("send-authorization-{recipient_id}");
        let expo_token = format!("ExponentPushToken[send-authorization-{recipient_id}]");
        let installation_id =
            insert_installation(pool, recipient_id, &public_installation_id, &expo_token).await?;
        let occurrence_id = insert_claimed_occurrence(
            pool,
            notification_id,
            source_event_id,
            message_id,
            recipient_id,
            installation_id,
            conversation_id,
        )
        .await?;
        Ok(Self {
            owner_id,
            recipient_id,
            group_id,
            conversation_id,
            message_id,
            notification_id,
            installation_id,
            public_installation_id,
            expo_token,
            occurrence_id,
            claim: PushDeliveryClaim {
                occurrence_id,
                claim_owner: CLAIM_OWNER.to_owned(),
                claim_generation: 1,
            },
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

async fn insert_group(pool: &PgPool, owner_id: Uuid, recipient_id: Uuid) -> TestResult<Uuid> {
    let group_id = Uuid::new_v4();
    sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, 'authorization', $2)")
        .bind(group_id)
        .bind(owner_id)
        .execute(pool)
        .await?;
    for (user_id, role) in [(owner_id, "owner"), (recipient_id, "member")] {
        sqlx::query(
            "INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, $4)",
        )
        .bind(Uuid::new_v4())
        .bind(group_id)
        .bind(user_id)
        .bind(role)
        .execute(pool)
        .await?;
    }
    sqlx::query(
        "INSERT INTO chatrooms (id, group_id, type, topic_id) VALUES ($1, $2, 'main', NULL)",
    )
    .bind(Uuid::new_v4())
    .bind(group_id)
    .execute(pool)
    .await?;
    Ok(group_id)
}

async fn insert_topic(pool: &PgPool, group_id: Uuid, owner_id: Uuid) -> TestResult<(Uuid, Uuid)> {
    let topic_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO topics \
             (id, group_id, author_id, idempotency_key, request_fingerprint, title) \
         VALUES ($1, $2, $3, $4, $5, 'authorization topic')",
    )
    .bind(topic_id)
    .bind(group_id)
    .bind(owner_id)
    .bind(Uuid::new_v4())
    .bind("a".repeat(64))
    .execute(pool)
    .await?;
    let conversation_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO chatrooms (id, group_id, type, topic_id) VALUES ($1, $2, 'topic', $3)",
    )
    .bind(conversation_id)
    .bind(group_id)
    .bind(topic_id)
    .execute(pool)
    .await?;
    Ok((topic_id, conversation_id))
}

async fn insert_message_event(
    pool: &PgPool,
    conversation_id: Uuid,
    sender_id: Uuid,
) -> TestResult<(Uuid, Uuid, i64)> {
    let message_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO messages (id, chatroom_id, sender_id, client_msg_id, body, type) \
         VALUES ($1, $2, $3, $4, $5, 'user')",
    )
    .bind(message_id)
    .bind(conversation_id)
    .bind(sender_id)
    .bind(Uuid::new_v4())
    .bind(MESSAGE_BODY)
    .execute(pool)
    .await?;
    let event_id = Uuid::new_v4();
    let cursor = sqlx::query_scalar::<_, i64>(
        "INSERT INTO conversation_events \
             (id, conversation_id, event_type, event_version, payload) \
         VALUES ($1, $2, 'message.created', 1, $3) RETURNING cursor",
    )
    .bind(event_id)
    .bind(conversation_id)
    .bind(json!({"id": message_id}))
    .fetch_one(pool)
    .await?;
    Ok((message_id, event_id, cursor))
}

async fn insert_notification(
    pool: &PgPool,
    recipient_id: Uuid,
    topic_id: Uuid,
    conversation_id: Uuid,
    source_cursor: i64,
) -> TestResult<Uuid> {
    let notification_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO notifications \
             (id, user_id, topic_id, conversation_id, source_cursor, type, payload, dedup_key) \
         VALUES ($1, $2, $3, $4, $5, 'chat_unread', $6, $7)",
    )
    .bind(notification_id)
    .bind(recipient_id)
    .bind(topic_id)
    .bind(conversation_id)
    .bind(source_cursor)
    .bind(json!({"sender_display_name": "authorization sender"}))
    .bind(format!("chat_unread:{topic_id}"))
    .execute(pool)
    .await?;
    Ok(notification_id)
}

async fn insert_installation(
    pool: &PgPool,
    recipient_id: Uuid,
    public_installation_id: &str,
    expo_token: &str,
) -> TestResult<Uuid> {
    let installation_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO push_installations \
             (id, user_id, installation_id, platform, provider, token, environment, \
              message_preview_enabled) \
         VALUES ($1, $2, $3, 'ios', 'expo', $4, 'development', true)",
    )
    .bind(installation_id)
    .bind(recipient_id)
    .bind(public_installation_id)
    .bind(expo_token)
    .execute(pool)
    .await?;
    Ok(installation_id)
}

async fn insert_claimed_occurrence(
    pool: &PgPool,
    notification_id: Uuid,
    source_event_id: Uuid,
    message_id: Uuid,
    recipient_id: Uuid,
    installation_id: Uuid,
    conversation_id: Uuid,
) -> TestResult<Uuid> {
    let occurrence_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO push_delivery_intents \
             (id, notification_id, source_event_id, source_message_id, recipient_user_id, \
              push_installation_id, installation_owner_epoch, \
              message_preview_enabled_snapshot, payload, status, claim_owner, \
              claim_generation, lease_expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, 1, true, $7, 'claimed', $8, 1, \
                 clock_timestamp() + INTERVAL '10 minutes')",
    )
    .bind(occurrence_id)
    .bind(notification_id)
    .bind(source_event_id)
    .bind(message_id)
    .bind(recipient_id)
    .bind(installation_id)
    .bind(json!({
        "type": "chat_unread",
        "notification_id": notification_id,
        "conversation_id": conversation_id,
        "message_id": message_id,
    }))
    .bind(CLAIM_OWNER)
    .execute(pool)
    .await?;
    Ok(occurrence_id)
}
