#![allow(
    dead_code,
    reason = "each moderation test module uses a different subset of the shared harness"
)]

//! Shared moderation HTTP harness over the real PostgreSQL adapter.

use std::{
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode, header::AUTHORIZATION},
};
use jamye_server::{
    adapters::postgres::{
        moderation::PostgresModerationRepository, transactions::SqlxTransactionManager,
    },
    application::{
        auth::{
            AccessIdentity, AccessTokenVerifier, AuthenticationError, GatedAccessTokenVerifier,
        },
        moderation::{
            ModerationAccessGate, ModerationDependencies, ModerationService, ModerationSettings,
        },
    },
    domain::moderation::ContentFilter,
    ports::rate_limit::{RateLimitFuture, RateLimitOutcome, RateLimitRequest, RateLimiter},
    transport::http::moderation::{ModerationHttpState, router},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

pub const MASKED_TERM: &str = "secretword";

pub struct Harness {
    pub database: TestDatabase,
    pub pool: PgPool,
    pub router: Router,
    pub limiter: Arc<RecordingLimiter>,
    pub service: Arc<ModerationService>,
    pub repository: Arc<PostgresModerationRepository>,
}

impl Harness {
    pub async fn new() -> TestResult<Self> {
        Self::with_operators(Vec::new()).await
    }

    pub async fn with_operators(operator_account_ids: Vec<Uuid>) -> TestResult<Self> {
        let database = TestDatabase::migrated().await?;
        let pool = database.pool()?;
        let limiter = Arc::new(RecordingLimiter::default());
        let repository = Arc::new(PostgresModerationRepository::new(pool.clone()));
        let service = Arc::new(ModerationService::new(
            ModerationDependencies {
                transactions: Arc::new(SqlxTransactionManager::new(pool.clone())),
                repository: repository.clone(),
                rate_limiter: limiter.clone(),
            },
            ModerationSettings {
                content_filter: ContentFilter::from_terms([MASKED_TERM])?,
                operator_account_ids,
            },
        ));
        let verifier = Arc::new(GatedAccessTokenVerifier::new(
            Arc::new(TestAccessVerifier),
            Arc::new(ModerationAccessGate::new(repository.clone())),
        ));
        let router = router(ModerationHttpState::new(service.clone(), verifier));
        Ok(Self {
            database,
            pool,
            router,
            limiter,
            service,
            repository,
        })
    }

    pub async fn finish(self, result: TestResult) -> TestResult {
        self.pool.close().await;
        let cleanup = self.database.dispose().await;
        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(failure), Ok(())) => Err(failure),
            (Ok(()), Err(cleanup_failure)) => Err(cleanup_failure),
            (Err(failure), Err(cleanup_failure)) => Err(format!(
                "moderation test failed: {failure}; database cleanup also failed: {cleanup_failure}"
            )
            .into()),
        }
    }

    pub async fn send(&self, request: Request<Body>) -> TestResult<Response<Body>> {
        Ok(self.router.clone().oneshot(request).await?)
    }

    pub async fn report(&self, actor: Uuid, body: &Value) -> TestResult<Response<Body>> {
        self.send(json_request(
            "POST",
            "/api/v1/reports",
            Some(actor),
            Some(body),
        )?)
        .await
    }

    pub async fn put_block(&self, actor: Uuid, target: &str) -> TestResult<Response<Body>> {
        self.send(json_request(
            "PUT",
            &format!("/api/v1/me/blocks/{target}"),
            Some(actor),
            None,
        )?)
        .await
    }

    pub async fn delete_block(&self, actor: Uuid, target: &str) -> TestResult<Response<Body>> {
        self.send(json_request(
            "DELETE",
            &format!("/api/v1/me/blocks/{target}"),
            Some(actor),
            None,
        )?)
        .await
    }

    pub async fn list_blocks(&self, actor: Uuid) -> TestResult<Response<Body>> {
        self.send(json_request("GET", "/api/v1/me/blocks", Some(actor), None)?)
            .await
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(_) => panic!("moderation test mutex is poisoned"),
    }
}

#[derive(Default)]
pub struct RecordingLimiter {
    requests: Mutex<Vec<RateLimitRequest>>,
    deny: Mutex<bool>,
}

impl RecordingLimiter {
    pub fn requests(&self) -> Vec<RateLimitRequest> {
        lock(&self.requests).clone()
    }

    pub fn deny_all(&self) {
        *lock(&self.deny) = true;
    }
}

impl RateLimiter for RecordingLimiter {
    fn check<'a>(&'a self, request: &'a RateLimitRequest) -> RateLimitFuture<'a> {
        Box::pin(async move {
            lock(&self.requests).push(request.clone());
            if *lock(&self.deny) {
                Ok(RateLimitOutcome::Denied {
                    retry_after: Duration::from_secs(7),
                })
            } else {
                Ok(RateLimitOutcome::Allowed)
            }
        })
    }
}

pub struct TestAccessVerifier;

impl AccessTokenVerifier for TestAccessVerifier {
    fn verify(&self, token: &str) -> Result<AccessIdentity, AuthenticationError> {
        let actor_id = token
            .strip_prefix("mod-")
            .and_then(|value| Uuid::try_parse(value).ok())
            .ok_or(AuthenticationError)?;
        Ok(AccessIdentity::new(
            actor_id,
            Uuid::nil(),
            "moderation-test",
        ))
    }
}

pub fn json_request(
    method: &str,
    uri: &str,
    actor: Option<Uuid>,
    body: Option<&Value>,
) -> TestResult<Request<Body>> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(actor) = actor {
        builder = builder.header(AUTHORIZATION, format!("Bearer mod-{actor}"));
    }
    Ok(match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(body)?))?,
        None => builder.body(Body::empty())?,
    })
}

pub async fn json_body(response: Response<Body>) -> TestResult<Value> {
    let bytes = to_bytes(response.into_body(), 128 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub async fn empty_body(response: Response<Body>) -> TestResult<bool> {
    let bytes = to_bytes(response.into_body(), 1024).await?;
    Ok(bytes.is_empty())
}

/// Assert the uniform error envelope: status, code and a request id; returns the message.
pub async fn assert_error(
    response: Response<Body>,
    status: StatusCode,
    code: &str,
) -> TestResult<String> {
    assert_eq!(response.status(), status);
    let body = json_body(response).await?;
    assert_eq!(body["error"]["code"], code);
    assert!(body["error"]["details"].is_null());
    Uuid::try_parse(
        body["error"]["request_id"]
            .as_str()
            .ok_or("error response omitted request_id")?,
    )?;
    Ok(body["error"]["message"]
        .as_str()
        .ok_or("error response omitted message")?
        .to_owned())
}

pub fn report_message_body(message_id: Uuid, reason: &str) -> Value {
    json!({"target_type": "message", "message_id": message_id, "reason": reason})
}

pub fn report_user_body(user_id: Uuid, reason: &str) -> Value {
    json!({"target_type": "user", "user_id": user_id, "reason": reason})
}

pub async fn insert_user(pool: &PgPool, nickname: &str) -> TestResult<Uuid> {
    insert_user_with_id(pool, Uuid::new_v4(), nickname).await
}

pub async fn insert_user_with_id(pool: &PgPool, user_id: Uuid, nickname: &str) -> TestResult<Uuid> {
    sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, $2)")
        .bind(user_id)
        .bind(nickname)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO auth_identities (id, user_id, provider, provider_id) \
         VALUES ($1, $2, 'kakao', $3)",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(format!("moderation-{user_id}"))
    .execute(pool)
    .await?;
    Ok(user_id)
}

#[derive(Clone, Copy, Debug)]
pub struct GroupFixture {
    pub group_id: Uuid,
    pub chatroom_id: Uuid,
}

/// A live group with a main chatroom; the first user owns it, the rest are members.
pub async fn insert_group(pool: &PgPool, members: &[Uuid]) -> TestResult<GroupFixture> {
    let owner = *members.first().ok_or("a group needs at least one member")?;
    let fixture = GroupFixture {
        group_id: Uuid::new_v4(),
        chatroom_id: Uuid::new_v4(),
    };
    sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, $2, $3)")
        .bind(fixture.group_id)
        .bind("moderation group")
        .bind(owner)
        .execute(pool)
        .await?;
    for (index, member) in members.iter().enumerate() {
        sqlx::query(
            "INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, $4)",
        )
        .bind(Uuid::new_v4())
        .bind(fixture.group_id)
        .bind(member)
        .bind(if index == 0 { "owner" } else { "member" })
        .execute(pool)
        .await?;
    }
    sqlx::query("INSERT INTO chatrooms (id, group_id, type) VALUES ($1, $2, 'main')")
        .bind(fixture.chatroom_id)
        .bind(fixture.group_id)
        .execute(pool)
        .await?;
    Ok(fixture)
}

pub async fn insert_message(
    pool: &PgPool,
    chatroom_id: Uuid,
    sender_id: Uuid,
    body: &str,
) -> TestResult<Uuid> {
    let message_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO messages (id, chatroom_id, sender_id, client_msg_id, body, type) \
         VALUES ($1, $2, $3, $4, $5, 'user')",
    )
    .bind(message_id)
    .bind(chatroom_id)
    .bind(sender_id)
    .bind(Uuid::new_v4())
    .bind(body)
    .execute(pool)
    .await?;
    Ok(message_id)
}

pub async fn insert_installation(pool: &PgPool, user_id: Uuid) -> TestResult<Uuid> {
    let installation_row_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO push_installations \
             (id, user_id, installation_id, platform, token, environment) \
         VALUES ($1, $2, $3, 'ios', $4, 'development')",
    )
    .bind(installation_row_id)
    .bind(user_id)
    .bind(format!("installation-{installation_row_id}"))
    .bind(format!("ExponentPushToken[{installation_row_id}]"))
    .execute(pool)
    .await?;
    Ok(installation_row_id)
}

pub async fn count(pool: &PgPool, sql: &'static str, id: Uuid) -> TestResult<i64> {
    Ok(sqlx::query_scalar::<_, i64>(sql)
        .bind(id)
        .fetch_one(pool)
        .await?)
}
