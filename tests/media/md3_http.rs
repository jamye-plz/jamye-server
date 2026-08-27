use std::sync::Arc;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode, header::AUTHORIZATION},
};
use jamye_server::{
    adapters::postgres::{topics::PostgresTopicsRepository, transactions::SqlxTransactionManager},
    application::{
        auth::{AccessIdentity, AccessTokenVerifier, AuthenticationError},
        topics::{TopicsDependencies, TopicsService},
    },
    transport::http::topics::{TopicsHttpState, router as topics_router},
};
use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;
use tower::ServiceExt;
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

const TOKEN_PREFIX: &str = "task8-md3-";

#[tokio::test]
async fn md3_returns_stable_paginated_canonical_topic_media() -> TestResult {
    let fixture = Md3Fixture::new().await?;
    let third = fixture
        .insert_topic_media(
            fixture.topic_id,
            OffsetDateTime::UNIX_EPOCH + time::Duration::hours(3),
            3_072,
        )
        .await?;
    let first = fixture
        .insert_topic_media(
            fixture.topic_id,
            OffsetDateTime::UNIX_EPOCH + time::Duration::hours(1),
            1_024,
        )
        .await?;
    let second = fixture
        .insert_topic_media(
            fixture.topic_id,
            OffsetDateTime::UNIX_EPOCH + time::Duration::hours(2),
            2_048,
        )
        .await?;
    let router = md3_router(fixture.pool.clone());

    let first_page = router
        .clone()
        .oneshot(get_request(
            &format!("/api/v1/topics/{}/media?limit=2", fixture.topic_id),
            Some(fixture.actor_id),
        )?)
        .await?;
    assert_eq!(first_page.status(), StatusCode::OK);
    let first_page = response_json(first_page).await?;
    let first_items = first_page["items"]
        .as_array()
        .ok_or("MD3 items must be an array")?;
    assert_eq!(first_items.len(), 2);
    assert_topic_media(&first_items[0], &first, fixture.topic_id);
    assert_topic_media(&first_items[1], &second, fixture.topic_id);
    assert_eq!(first_page["next_cursor"], second.id.to_string());
    assert_eq!(first_page.as_object().map(|page| page.len()), Some(2));

    let second_page = router
        .oneshot(get_request(
            &format!(
                "/api/v1/topics/{}/media?after={}&limit=2",
                fixture.topic_id, second.id
            ),
            Some(fixture.actor_id),
        )?)
        .await?;
    assert_eq!(second_page.status(), StatusCode::OK);
    let second_page = response_json(second_page).await?;
    let second_items = second_page["items"]
        .as_array()
        .ok_or("MD3 items must be an array")?;
    assert_eq!(second_items.len(), 1);
    assert_topic_media(&second_items[0], &third, fixture.topic_id);
    assert!(second_page["next_cursor"].is_null());

    fixture.dispose().await
}

#[tokio::test]
async fn md3_requires_bearer_auth_and_rejects_malformed_path_or_query() -> TestResult {
    let fixture = Md3Fixture::new().await?;
    let router = md3_router(fixture.pool.clone());
    let uri = format!("/api/v1/topics/{}/media", fixture.topic_id);

    let unauthorized = router.clone().oneshot(get_request(&uri, None)?).await?;
    assert_error(
        unauthorized,
        StatusCode::UNAUTHORIZED,
        "authentication_required",
    )
    .await?;

    for invalid_uri in [
        "/api/v1/topics/not-a-uuid/media".to_owned(),
        format!("{uri}?after=not-a-uuid"),
        format!("{uri}?limit=0"),
        format!("{uri}?limit=101"),
        format!("{uri}?after={0}&after={0}", Uuid::new_v4()),
        format!("{uri}?before={}", Uuid::new_v4()),
    ] {
        let response = router
            .clone()
            .oneshot(get_request(&invalid_uri, Some(fixture.actor_id))?)
            .await?;
        assert_error(
            response,
            StatusCode::UNPROCESSABLE_ENTITY,
            "request_validation_failed",
        )
        .await?;
    }

    fixture.dispose().await
}

#[tokio::test]
async fn md3_missing_nonmember_and_cross_topic_cursor_share_one_bola_envelope() -> TestResult {
    let fixture = Md3Fixture::new().await?;
    let secret = fixture
        .insert_topic_media(
            fixture.topic_id,
            OffsetDateTime::UNIX_EPOCH + time::Duration::hours(1),
            1_024,
        )
        .await?;
    let foreign = fixture
        .insert_topic_media(
            fixture.foreign_topic_id,
            OffsetDateTime::UNIX_EPOCH + time::Duration::hours(2),
            2_048,
        )
        .await?;
    let router = md3_router(fixture.pool.clone());
    let requests = [
        get_request(
            &format!("/api/v1/topics/{}/media", fixture.topic_id),
            Some(fixture.outsider_id),
        )?,
        get_request(
            &format!("/api/v1/topics/{}/media", Uuid::new_v4()),
            Some(fixture.actor_id),
        )?,
        get_request(
            &format!(
                "/api/v1/topics/{}/media?after={}",
                fixture.topic_id, foreign.id
            ),
            Some(fixture.actor_id),
        )?,
    ];

    for request in requests {
        let response = router.clone().oneshot(request).await?;
        let body = assert_error(response, StatusCode::FORBIDDEN, "membership_required").await?;
        let serialized = serde_json::to_string(&body)?;
        for private_value in [
            fixture.topic_id.to_string(),
            secret.id.to_string(),
            secret.object_key.clone(),
        ] {
            assert!(!serialized.contains(&private_value));
        }
    }

    fixture.dispose().await
}

fn assert_topic_media(value: &Value, expected: &SeededTopicMedia, topic_id: Uuid) {
    assert_eq!(value["id"], expected.id.to_string());
    assert_eq!(value["topic_id"], topic_id.to_string());
    assert_eq!(value["media_upload_id"], expected.upload_id.to_string());
    assert_eq!(value["content_type"], "image/jpeg");
    assert_eq!(value["object_key"], expected.object_key);
    assert_eq!(value["width"], 800);
    assert_eq!(value["height"], 600);
    assert_eq!(value["byte_size"], expected.byte_size);
    assert!(value["created_at"].as_str().is_some());
    assert_eq!(value.as_object().map(|media| media.len()), Some(9));
}

fn md3_router(pool: PgPool) -> Router {
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

fn get_request(uri: &str, actor_id: Option<Uuid>) -> TestResult<Request<Body>> {
    let mut request = Request::get(uri);
    if let Some(actor_id) = actor_id {
        request = request.header(AUTHORIZATION, format!("Bearer {TOKEN_PREFIX}{actor_id}"));
    }
    Ok(request.body(Body::empty())?)
}

async fn assert_error(
    response: Response<Body>,
    status: StatusCode,
    code: &str,
) -> TestResult<Value> {
    assert_eq!(response.status(), status);
    let body = response_json(response).await?;
    assert_eq!(body["error"]["code"], code);
    assert!(body["error"]["details"].is_null());
    Uuid::try_parse(
        body["error"]["request_id"]
            .as_str()
            .ok_or("error response omitted request_id")?,
    )?;
    assert_eq!(body.as_object().map(|object| object.len()), Some(1));
    Ok(body)
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
            "task-8-md3-test",
        ))
    }
}

struct Md3Fixture {
    database: TestDatabase,
    pool: PgPool,
    actor_id: Uuid,
    outsider_id: Uuid,
    topic_id: Uuid,
    foreign_topic_id: Uuid,
}

impl Md3Fixture {
    async fn new() -> TestResult<Self> {
        let database = TestDatabase::migrated().await?;
        let pool = database.pool()?;
        let actor_id = insert_user(&pool, "MD3 멤버").await?;
        let outsider_id = insert_user(&pool, "MD3 외부인").await?;
        let group_id = insert_group(&pool, actor_id, "MD3 그룹").await?;
        insert_membership(&pool, group_id, actor_id, "owner").await?;
        let foreign_group_id = insert_group(&pool, outsider_id, "MD3 외부 그룹").await?;
        insert_membership(&pool, foreign_group_id, outsider_id, "owner").await?;
        let topic_id = insert_topic(&pool, group_id, actor_id, "MD3 주제").await?;
        let foreign_topic_id =
            insert_topic(&pool, foreign_group_id, outsider_id, "MD3 외부 주제").await?;
        Ok(Self {
            database,
            pool,
            actor_id,
            outsider_id,
            topic_id,
            foreign_topic_id,
        })
    }

    async fn insert_topic_media(
        &self,
        topic_id: Uuid,
        created_at: OffsetDateTime,
        byte_size: i64,
    ) -> TestResult<SeededTopicMedia> {
        let id = Uuid::new_v4();
        let upload_id = Uuid::new_v4();
        let object_key = format!("topics/{topic_id}/{upload_id}");
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO media_uploads \
                 (id, user_id, object_key, scope, target_id, content_type, byte_size, \
                  filename, status, bound_topic_media_id, confirmed_at, consumed_at, \
                  expires_at, created_at) \
             VALUES ($1, $2, $3, 'topic', $4, 'image/jpeg', $5, 'topic.jpg', \
                     'bound', $6, $7, $8, $9, $10)",
        )
        .bind(upload_id)
        .bind(if topic_id == self.topic_id {
            self.actor_id
        } else {
            self.outsider_id
        })
        .bind(&object_key)
        .bind(topic_id)
        .bind(byte_size)
        .bind(id)
        .bind(created_at - time::Duration::minutes(2))
        .bind(created_at - time::Duration::minutes(1))
        .bind(created_at + time::Duration::hours(1))
        .bind(created_at - time::Duration::minutes(3))
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO topic_media \
                 (id, topic_id, media_upload_id, type, object_key, width, height, \
                  byte_size, created_at) \
             VALUES ($1, $2, $3, 'image/jpeg', $4, 800, 600, $5, $6)",
        )
        .bind(id)
        .bind(topic_id)
        .bind(upload_id)
        .bind(&object_key)
        .bind(byte_size)
        .bind(created_at)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(SeededTopicMedia {
            id,
            upload_id,
            object_key,
            byte_size,
        })
    }

    async fn dispose(self) -> TestResult {
        self.pool.close().await;
        self.database.dispose().await
    }
}

struct SeededTopicMedia {
    id: Uuid,
    upload_id: Uuid,
    object_key: String,
    byte_size: i64,
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

async fn insert_group(pool: &PgPool, owner_id: Uuid, name: &str) -> TestResult<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(name)
        .bind(owner_id)
        .execute(pool)
        .await?;
    Ok(id)
}

async fn insert_membership(pool: &PgPool, group_id: Uuid, user_id: Uuid, role: &str) -> TestResult {
    sqlx::query("INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, $4)")
        .bind(Uuid::new_v4())
        .bind(group_id)
        .bind(user_id)
        .bind(role)
        .execute(pool)
        .await?;
    Ok(())
}

async fn insert_topic(
    pool: &PgPool,
    group_id: Uuid,
    author_id: Uuid,
    title: &str,
) -> TestResult<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO topics \
             (id, group_id, author_id, idempotency_key, request_fingerprint, title, status) \
         VALUES ($1, $2, $3, $4, $5, $6, 'enriched')",
    )
    .bind(id)
    .bind(group_id)
    .bind(author_id)
    .bind(Uuid::new_v4())
    .bind("0".repeat(64))
    .bind(title)
    .execute(pool)
    .await?;
    Ok(id)
}
