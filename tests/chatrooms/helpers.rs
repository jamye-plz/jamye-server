use std::sync::Arc;

use jamye_server::{
    adapters::postgres::{
        chatrooms::PostgresChatroomsRepository, transactions::SqlxTransactionManager,
    },
    application::{
        auth::{AccessIdentity, AccessTokenVerifier, AuthenticationError},
        chatrooms::ChatroomsService,
    },
};
use serde_json::json;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::TestResult;

pub struct ChatroomsHarness {
    pub service: Arc<ChatroomsService>,
}

pub struct Topology {
    pub owner_id: Uuid,
    pub outsider_id: Uuid,
    pub group_id: Uuid,
    pub chatroom_id: Uuid,
    pub other_chatroom_id: Uuid,
}

pub fn harness(pool: PgPool) -> ChatroomsHarness {
    let repository = Arc::new(PostgresChatroomsRepository::new(pool.clone()));
    let transactions = Arc::new(SqlxTransactionManager::new(pool));
    ChatroomsHarness {
        service: Arc::new(ChatroomsService::new(transactions, repository)),
    }
}

pub async fn topology(pool: &PgPool) -> TestResult<Topology> {
    let owner_id = insert_user(pool, "채팅방 소유자", Some("https://cdn.test/owner.png")).await?;
    let outsider_id = insert_user(pool, "외부 사용자", None).await?;
    let group_id = insert_group(pool, owner_id, "채팅방 그룹").await?;
    insert_member(pool, group_id, owner_id, "owner").await?;
    let chatroom_id = insert_chatroom(
        pool,
        group_id,
        "main",
        None,
        OffsetDateTime::UNIX_EPOCH + time::Duration::hours(1),
    )
    .await?;

    let other_group_id = insert_group(pool, outsider_id, "다른 그룹").await?;
    insert_member(pool, other_group_id, outsider_id, "owner").await?;
    let other_chatroom_id = insert_chatroom(
        pool,
        other_group_id,
        "main",
        None,
        OffsetDateTime::UNIX_EPOCH + time::Duration::hours(2),
    )
    .await?;

    Ok(Topology {
        owner_id,
        outsider_id,
        group_id,
        chatroom_id,
        other_chatroom_id,
    })
}

pub async fn insert_user(
    pool: &PgPool,
    nickname: &str,
    avatar_url: Option<&str>,
) -> TestResult<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, nickname, avatar_url) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(nickname)
        .bind(avatar_url)
        .execute(pool)
        .await?;
    Ok(id)
}

pub async fn insert_group(pool: &PgPool, owner_id: Uuid, name: &str) -> TestResult<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(name)
        .bind(owner_id)
        .execute(pool)
        .await?;
    Ok(id)
}

pub async fn insert_member(
    pool: &PgPool,
    group_id: Uuid,
    user_id: Uuid,
    role: &str,
) -> TestResult<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, $4)")
        .bind(id)
        .bind(group_id)
        .bind(user_id)
        .bind(role)
        .execute(pool)
        .await?;
    Ok(id)
}

pub async fn insert_chatroom(
    pool: &PgPool,
    group_id: Uuid,
    chatroom_type: &str,
    topic_id: Option<Uuid>,
    created_at: OffsetDateTime,
) -> TestResult<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO chatrooms (id, group_id, type, topic_id, created_at) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(group_id)
    .bind(chatroom_type)
    .bind(topic_id)
    .bind(created_at)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn insert_user_message(
    pool: &PgPool,
    chatroom_id: Uuid,
    sender_id: Uuid,
    body: &str,
    created_at: OffsetDateTime,
) -> TestResult<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO messages \
             (id, chatroom_id, sender_id, client_msg_id, body, type, created_at) \
         VALUES ($1, $2, $3, $4, $5, 'user', $6)",
    )
    .bind(id)
    .bind(chatroom_id)
    .bind(sender_id)
    .bind(Uuid::new_v4())
    .bind(body)
    .bind(created_at)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn insert_system_message(
    pool: &PgPool,
    chatroom_id: Uuid,
    body: &str,
    created_at: OffsetDateTime,
) -> TestResult<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO messages (id, chatroom_id, body, type, created_at) \
         VALUES ($1, $2, $3, 'system', $4)",
    )
    .bind(id)
    .bind(chatroom_id)
    .bind(body)
    .bind(created_at)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn insert_event(pool: &PgPool, chatroom_id: Uuid) -> TestResult<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "INSERT INTO conversation_events \
             (id, conversation_id, event_type, event_version, payload) \
         VALUES ($1, $2, 'message.created', 1, $3) \
         RETURNING cursor",
    )
    .bind(Uuid::new_v4())
    .bind(chatroom_id)
    .bind(json!({"fixture": "task-6b"}))
    .fetch_one(pool)
    .await?)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TestAccessVerifier;

impl AccessTokenVerifier for TestAccessVerifier {
    fn verify(&self, token: &str) -> Result<AccessIdentity, AuthenticationError> {
        let user_id = token
            .strip_prefix("task6b-")
            .and_then(|value| Uuid::try_parse(value).ok())
            .ok_or(AuthenticationError)?;
        Ok(AccessIdentity::new(user_id, Uuid::nil(), "task-6b-test"))
    }
}

pub fn bearer(user_id: Uuid) -> String {
    format!("Bearer task6b-{user_id}")
}
