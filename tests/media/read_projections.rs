use std::sync::Arc;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode, header::AUTHORIZATION},
};
use jamye_server::{
    adapters::postgres::{
        chatrooms::PostgresChatroomsRepository, topics::PostgresTopicsRepository,
        transactions::SqlxTransactionManager,
    },
    application::{
        auth::{AccessIdentity, AccessTokenVerifier, AuthenticationError},
        chatrooms::ChatroomsService,
        topics::{TopicsDependencies, TopicsService},
    },
    transport::http::{
        chatrooms::{ChatroomsHttpState, router as chatrooms_router},
        topics::{TopicsHttpState, router as topics_router},
    },
};
use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;
use tower::ServiceExt;
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

const TOKEN_PREFIX: &str = "task8-read-";

#[tokio::test]
async fn history_projects_persisted_media_in_position_order_without_object_keys() -> TestResult {
    let fixture = ProjectionFixture::new().await?;
    let later = fixture
        .insert_message_media(MessageMediaSpec {
            content_type: "video/mp4",
            byte_size: 4_096,
            width: None,
            height: None,
            filename: Some("clip.mp4"),
            position: 1,
        })
        .await?;
    let first = fixture
        .insert_message_media(MessageMediaSpec {
            content_type: "image/jpeg",
            byte_size: 1_024,
            width: Some(800),
            height: Some(600),
            filename: Some(" 여름/기록.jpg "),
            position: 0,
        })
        .await?;
    let response = history_router(fixture.pool.clone())
        .oneshot(get_request(
            &format!(
                "/api/v1/chatrooms/{}/messages?limit=10",
                fixture.chatroom_id
            ),
            fixture.actor_id,
        )?)
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await?;
    let media = body["items"][0]["media"]
        .as_array()
        .ok_or("history media must be an array")?;
    assert_eq!(media.len(), 2);
    assert_eq!(media[0]["id"], first.media_id.to_string());
    assert_eq!(media[0]["media_upload_id"], first.upload_id.to_string());
    assert_eq!(media[0]["type"], "image/jpeg");
    assert_eq!(media[0]["byte_size"], 1_024);
    assert_eq!(media[0]["width"], 800);
    assert_eq!(media[0]["height"], 600);
    assert_eq!(media[0]["duration"], Value::Null);
    assert_eq!(media[0]["filename"], " 여름/기록.jpg ");
    assert_eq!(media[0]["position"], 0);
    assert!(media[0].get("object_key").is_none());
    assert_eq!(media[1]["id"], later.media_id.to_string());
    assert_eq!(media[1]["media_upload_id"], later.upload_id.to_string());
    assert_eq!(media[1]["type"], "video/mp4");
    assert_eq!(media[1]["position"], 1);
    assert!(media[1].get("object_key").is_none());

    fixture.dispose().await
}

#[tokio::test]
async fn md3_projects_the_canonical_topic_media_upload_identity() -> TestResult {
    let fixture = ProjectionFixture::new().await?;
    let media = fixture.insert_topic_media().await?;
    let response = topic_router(fixture.pool.clone())
        .oneshot(get_request(
            &format!(
                "/api/v1/groups/{}/topics/{}",
                fixture.group_id, fixture.topic_id
            ),
            fixture.actor_id,
        )?)
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await?;
    let projected = &body["media"][0];
    assert_eq!(projected["id"], media.media_id.to_string());
    assert_eq!(projected["topic_id"], fixture.topic_id.to_string());
    assert_eq!(projected["media_upload_id"], media.upload_id.to_string());
    assert_eq!(projected["content_type"], "image/jpeg");
    assert_eq!(projected["width"], 800);
    assert_eq!(projected["height"], 600);
    assert_eq!(projected["byte_size"], 1_024);

    fixture.dispose().await
}

fn history_router(pool: PgPool) -> Router {
    let repository = Arc::new(PostgresChatroomsRepository::new(pool.clone()));
    let transactions = Arc::new(SqlxTransactionManager::new(pool));
    chatrooms_router(ChatroomsHttpState::new(
        Arc::new(ChatroomsService::new(transactions, repository)),
        Arc::new(TestAccessVerifier),
    ))
}

fn topic_router(pool: PgPool) -> Router {
    let repository = Arc::new(PostgresTopicsRepository::new(pool.clone()));
    let transactions = Arc::new(SqlxTransactionManager::new(pool));
    topics_router(TopicsHttpState::new(
        Arc::new(TopicsService::new(TopicsDependencies {
            transactions,
            repository,
        })),
        Arc::new(TestAccessVerifier),
    ))
}

fn get_request(uri: &str, actor_id: Uuid) -> TestResult<Request<Body>> {
    Ok(Request::get(uri)
        .header(AUTHORIZATION, format!("Bearer {TOKEN_PREFIX}{actor_id}"))
        .body(Body::empty())?)
}

async fn response_json(response: Response<Body>) -> TestResult<Value> {
    let bytes = to_bytes(response.into_body(), 128 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[derive(Clone, Copy, Debug, Default)]
struct TestAccessVerifier;

impl AccessTokenVerifier for TestAccessVerifier {
    fn verify(&self, token: &str) -> Result<AccessIdentity, AuthenticationError> {
        let actor_id = token
            .strip_prefix(TOKEN_PREFIX)
            .and_then(|value| Uuid::try_parse(value).ok())
            .ok_or(AuthenticationError)?;
        Ok(AccessIdentity::new(
            actor_id,
            Uuid::nil(),
            "task-8-read-test",
        ))
    }
}

struct ProjectionFixture {
    database: TestDatabase,
    pool: PgPool,
    actor_id: Uuid,
    group_id: Uuid,
    chatroom_id: Uuid,
    message_id: Uuid,
    topic_id: Uuid,
}

impl ProjectionFixture {
    async fn new() -> TestResult<Self> {
        let database = TestDatabase::migrated().await?;
        let pool = database.pool()?;
        let actor_id = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, '읽기 투영 사용자')")
            .bind(actor_id)
            .execute(&pool)
            .await?;

        let group_id = Uuid::new_v4();
        sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, '읽기 투영 그룹', $2)")
            .bind(group_id)
            .bind(actor_id)
            .execute(&pool)
            .await?;
        sqlx::query(
            "INSERT INTO memberships (id, group_id, user_id, role) \
             VALUES ($1, $2, $3, 'owner')",
        )
        .bind(Uuid::new_v4())
        .bind(group_id)
        .bind(actor_id)
        .execute(&pool)
        .await?;

        let chatroom_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO chatrooms (id, group_id, type, topic_id) \
             VALUES ($1, $2, 'main', NULL)",
        )
        .bind(chatroom_id)
        .bind(group_id)
        .execute(&pool)
        .await?;
        let message_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO messages \
                 (id, chatroom_id, sender_id, client_msg_id, body, type) \
             VALUES ($1, $2, $3, $4, '미디어 히스토리', 'user')",
        )
        .bind(message_id)
        .bind(chatroom_id)
        .bind(actor_id)
        .bind(Uuid::new_v4())
        .execute(&pool)
        .await?;

        let topic_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO topics \
                 (id, group_id, author_id, idempotency_key, request_fingerprint, title, status) \
             VALUES ($1, $2, $3, $4, $5, '미디어 주제', 'enriched')",
        )
        .bind(topic_id)
        .bind(group_id)
        .bind(actor_id)
        .bind(Uuid::new_v4())
        .bind("0".repeat(64))
        .execute(&pool)
        .await?;
        sqlx::query(
            "INSERT INTO chatrooms (id, group_id, type, topic_id) \
             VALUES ($1, $2, 'topic', $3)",
        )
        .bind(Uuid::new_v4())
        .bind(group_id)
        .bind(topic_id)
        .execute(&pool)
        .await?;

        Ok(Self {
            database,
            pool,
            actor_id,
            group_id,
            chatroom_id,
            message_id,
            topic_id,
        })
    }

    async fn insert_message_media(&self, spec: MessageMediaSpec<'_>) -> TestResult<SeededMedia> {
        let media_id = Uuid::new_v4();
        let upload_id = Uuid::new_v4();
        let object_key = format!("chat/{}/{upload_id}", self.chatroom_id);
        let now = OffsetDateTime::now_utc();
        sqlx::query(
            "INSERT INTO media_uploads \
                 (id, user_id, object_key, scope, target_id, content_type, byte_size, \
                  filename, status, bound_message_id, confirmed_at, consumed_at, \
                  expires_at, created_at) \
             VALUES ($1, $2, $3, 'chat', $4, $5, $6, $7, 'bound', $8, $9, $10, $11, $12)",
        )
        .bind(upload_id)
        .bind(self.actor_id)
        .bind(&object_key)
        .bind(self.chatroom_id)
        .bind(spec.content_type)
        .bind(spec.byte_size)
        .bind(spec.filename)
        .bind(self.message_id)
        .bind(now - time::Duration::minutes(2))
        .bind(now - time::Duration::minutes(1))
        .bind(now + time::Duration::hours(1))
        .bind(now - time::Duration::hours(1))
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "INSERT INTO message_media \
                 (id, message_id, media_upload_id, type, object_key, width, height, \
                  byte_size, position, filename) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(media_id)
        .bind(self.message_id)
        .bind(upload_id)
        .bind(spec.content_type)
        .bind(object_key)
        .bind(spec.width)
        .bind(spec.height)
        .bind(spec.byte_size)
        .bind(spec.position)
        .bind(spec.filename)
        .execute(&self.pool)
        .await?;
        Ok(SeededMedia {
            media_id,
            upload_id,
        })
    }

    async fn insert_topic_media(&self) -> TestResult<SeededMedia> {
        let media_id = Uuid::new_v4();
        let upload_id = Uuid::new_v4();
        let object_key = format!("topics/{}/{upload_id}", self.topic_id);
        let now = OffsetDateTime::now_utc();
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO media_uploads \
                 (id, user_id, object_key, scope, target_id, content_type, byte_size, \
                  filename, status, bound_topic_media_id, confirmed_at, consumed_at, \
                  expires_at, created_at) \
             VALUES ($1, $2, $3, 'topic', $4, 'image/jpeg', 1024, 'topic.jpg', \
                     'bound', $5, $6, $7, $8, $9)",
        )
        .bind(upload_id)
        .bind(self.actor_id)
        .bind(&object_key)
        .bind(self.topic_id)
        .bind(media_id)
        .bind(now - time::Duration::minutes(2))
        .bind(now - time::Duration::minutes(1))
        .bind(now + time::Duration::hours(1))
        .bind(now - time::Duration::hours(1))
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO topic_media \
                 (id, topic_id, media_upload_id, type, object_key, width, height, byte_size) \
             VALUES ($1, $2, $3, 'image/jpeg', $4, 800, 600, 1024)",
        )
        .bind(media_id)
        .bind(self.topic_id)
        .bind(upload_id)
        .bind(object_key)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(SeededMedia {
            media_id,
            upload_id,
        })
    }

    async fn dispose(self) -> TestResult {
        self.pool.close().await;
        self.database.dispose().await
    }
}

#[derive(Clone, Copy)]
struct MessageMediaSpec<'a> {
    content_type: &'a str,
    byte_size: i64,
    width: Option<i32>,
    height: Option<i32>,
    filename: Option<&'a str>,
    position: i32,
}

#[derive(Clone, Copy)]
struct SeededMedia {
    media_id: Uuid,
    upload_id: Uuid,
}
