#![allow(
    dead_code,
    reason = "each avatar test module uses a different subset of the shared harness"
)]

//! Shared avatar HTTP harness over the real PostgreSQL adapter and in-memory object storage.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, Response, StatusCode, header::AUTHORIZATION},
};
use jamye_server::{
    adapters::postgres::{avatar::PostgresAvatarRepository, transactions::SqlxTransactionManager},
    application::{
        auth::{AccessIdentity, AccessTokenVerifier, AuthenticationError},
        avatar::{AvatarDependencies, AvatarEndpointRateLimit, AvatarService, AvatarSettings},
        media::MediaEndpointRateLimit,
    },
    domain::media::InspectedObject,
    ports::{
        object_storage::{
            InspectObjectRequest, MediaObjectStorage, MediaObjectStorageFuture,
            ObjectStorageProviderError, PresignPutRequest, PresignedPut,
        },
        rate_limit::{RateLimitFuture, RateLimitOutcome, RateLimitRequest, RateLimiter},
    },
    transport::http::avatar::{AvatarHttpState, router},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

pub const BASE: &str = "https://avatars.example.test";
pub const UPLOAD_LIMIT: u32 = 2;
pub const READ_LIMIT: u32 = 3;
pub const PEER_IP: &str = "203.0.113.9";

pub struct Harness {
    pub database: TestDatabase,
    pub pool: PgPool,
    pub router: Router,
    pub storage: Arc<FakeObjectStorage>,
    pub limiter: Arc<RecordingLimiter>,
}

impl Harness {
    pub async fn new() -> TestResult<Self> {
        let database = TestDatabase::migrated().await?;
        let pool = database.pool()?;
        let storage = Arc::new(FakeObjectStorage::default());
        let limiter = Arc::new(RecordingLimiter::default());
        let service = AvatarService::new(
            AvatarDependencies {
                transactions: Arc::new(SqlxTransactionManager::new(pool.clone())),
                repository: Arc::new(PostgresAvatarRepository::new(pool.clone())),
                object_storage: storage.clone(),
                rate_limiter: limiter.clone(),
            },
            AvatarSettings {
                public_base_url: BASE.to_owned(),
                upload_rate_limit: MediaEndpointRateLimit {
                    limit: UPLOAD_LIMIT,
                    window: Duration::from_secs(60),
                },
                public_read_rate_limit: AvatarEndpointRateLimit {
                    limit: READ_LIMIT,
                    window: Duration::from_secs(10),
                },
            },
        )?;
        let router = router(AvatarHttpState::new(
            Arc::new(service),
            Arc::new(TestAccessVerifier),
        ));
        Ok(Self {
            database,
            pool,
            router,
            storage,
            limiter,
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
                "avatar test failed: {failure}; database cleanup also failed: {cleanup_failure}"
            )
            .into()),
        }
    }

    pub async fn send(&self, request: Request<Body>) -> TestResult<Response<Body>> {
        Ok(self.router.clone().oneshot(request).await?)
    }

    /// POST U4 with a valid body and return the new upload id.
    pub async fn create_intent(&self, actor: Uuid, byte_size: u64) -> TestResult<Uuid> {
        let response = self
            .send(json_post(
                "/api/v1/me/avatar/uploads",
                Some(actor),
                &json!({"content_type": "image/jpeg", "byte_size": byte_size}),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = json_body(response).await?;
        Ok(Uuid::try_parse(
            body["upload_id"].as_str().ok_or("upload_id missing")?,
        )?)
    }

    /// POST U5 with the empty body.
    pub async fn finalize(&self, actor: Uuid, upload_id: Uuid) -> TestResult<Response<Body>> {
        self.send(json_post(
            &format!("/api/v1/me/avatar/uploads/{upload_id}/finalize"),
            Some(actor),
            &json!({}),
        )?)
        .await
    }

    /// U4 then store a valid JPEG then U5: returns the active upload id.
    pub async fn upload_and_finalize(&self, actor: Uuid) -> TestResult<Uuid> {
        let bytes = jpeg_bytes(2048);
        let upload_id = self.create_intent(actor, 2048).await?;
        self.storage
            .put(&object_key(actor, upload_id), "image/jpeg", bytes);
        let response = self.finalize(actor, upload_id).await?;
        assert_eq!(response.status(), StatusCode::OK);
        Ok(upload_id)
    }

    pub fn requests_for(&self, endpoint: &str) -> Vec<RateLimitRequest> {
        self.limiter
            .requests()
            .into_iter()
            .filter(|request| request.endpoint == endpoint)
            .collect()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(_) => panic!("avatar test mutex is poisoned"),
    }
}

pub struct StoredObject {
    pub content_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Default)]
pub struct FakeObjectStorage {
    objects: Mutex<HashMap<String, StoredObject>>,
    presigned: Mutex<Vec<PresignPutRequest>>,
    unavailable: AtomicBool,
    get_unavailable: AtomicBool,
}

impl FakeObjectStorage {
    pub fn put(&self, object_key: &str, content_type: &str, bytes: Vec<u8>) {
        lock(&self.objects).insert(
            object_key.to_owned(),
            StoredObject {
                content_type: content_type.to_owned(),
                bytes,
            },
        );
    }

    /// Fail presign and inspect with a provider outage.
    pub fn set_unavailable(&self, unavailable: bool) {
        self.unavailable.store(unavailable, Ordering::SeqCst);
    }

    /// Fail only the full-object read used by the public avatar route.
    pub fn set_get_unavailable(&self, unavailable: bool) {
        self.get_unavailable.store(unavailable, Ordering::SeqCst);
    }

    pub fn presigned_requests(&self) -> Vec<PresignPutRequest> {
        lock(&self.presigned).clone()
    }
}

impl MediaObjectStorage for FakeObjectStorage {
    fn presign_put<'a>(
        &'a self,
        request: &'a PresignPutRequest,
    ) -> MediaObjectStorageFuture<'a, PresignedPut> {
        Box::pin(async move {
            if self.unavailable.load(Ordering::SeqCst) {
                return Err(ObjectStorageProviderError::Unavailable);
            }
            lock(&self.presigned).push(request.clone());
            Ok(PresignedPut {
                url: format!(
                    "https://objects.example.test/{}?signature=fake",
                    request.object_key
                ),
                expires_in: request.expires_in,
            })
        })
    }

    fn inspect_object<'a>(
        &'a self,
        request: &'a InspectObjectRequest,
    ) -> MediaObjectStorageFuture<'a, InspectedObject> {
        Box::pin(async move {
            if self.unavailable.load(Ordering::SeqCst) {
                return Err(ObjectStorageProviderError::Unavailable);
            }
            let objects = lock(&self.objects);
            Ok(match objects.get(&request.object_key) {
                Some(object) => InspectedObject {
                    content_type: Some(object.content_type.clone()),
                    byte_size: u64::try_from(object.bytes.len()).ok(),
                    audio_duration: None,
                },
                None => InspectedObject {
                    content_type: None,
                    byte_size: None,
                    audio_duration: None,
                },
            })
        })
    }

    fn get_object<'a>(
        &'a self,
        object_key: &'a str,
        max_bytes: u64,
    ) -> MediaObjectStorageFuture<'a, Vec<u8>> {
        Box::pin(async move {
            if self.unavailable.load(Ordering::SeqCst)
                || self.get_unavailable.load(Ordering::SeqCst)
            {
                return Err(ObjectStorageProviderError::Unavailable);
            }
            let objects = lock(&self.objects);
            let object = objects
                .get(object_key)
                .ok_or(ObjectStorageProviderError::UnexpectedResponse)?;
            if !matches!(u64::try_from(object.bytes.len()), Ok(length) if length <= max_bytes) {
                return Err(ObjectStorageProviderError::UnexpectedResponse);
            }
            Ok(object.bytes.clone())
        })
    }
}

#[derive(Default)]
pub struct RecordingLimiter {
    requests: Mutex<Vec<RateLimitRequest>>,
    denied_endpoint: Mutex<Option<&'static str>>,
}

impl RecordingLimiter {
    pub fn requests(&self) -> Vec<RateLimitRequest> {
        lock(&self.requests).clone()
    }

    pub fn deny(&self, endpoint: &'static str) {
        *lock(&self.denied_endpoint) = Some(endpoint);
    }
}

impl RateLimiter for RecordingLimiter {
    fn check<'a>(&'a self, request: &'a RateLimitRequest) -> RateLimitFuture<'a> {
        Box::pin(async move {
            lock(&self.requests).push(request.clone());
            let denied = *lock(&self.denied_endpoint);
            if denied == Some(request.endpoint) {
                Ok(RateLimitOutcome::Denied {
                    retry_after: Duration::from_secs(7),
                })
            } else {
                Ok(RateLimitOutcome::Allowed)
            }
        })
    }
}

struct TestAccessVerifier;

impl AccessTokenVerifier for TestAccessVerifier {
    fn verify(&self, token: &str) -> Result<AccessIdentity, AuthenticationError> {
        let actor_id = token
            .strip_prefix("avatar-")
            .and_then(|value| Uuid::try_parse(value).ok())
            .ok_or(AuthenticationError)?;
        Ok(AccessIdentity::new(actor_id, Uuid::nil(), "avatar-test"))
    }
}

pub fn jpeg_bytes(length: usize) -> Vec<u8> {
    let mut bytes = vec![0x5A; length];
    for (index, byte) in [0xFF, 0xD8, 0xFF, 0xE0].into_iter().enumerate() {
        if let Some(slot) = bytes.get_mut(index) {
            *slot = byte;
        }
    }
    bytes
}

pub fn object_key(user_id: Uuid, upload_id: Uuid) -> String {
    format!("avatar/{user_id}/{upload_id}")
}

pub fn json_post(uri: &str, actor: Option<Uuid>, body: &Value) -> TestResult<Request<Body>> {
    let mut builder = Request::post(uri).header("content-type", "application/json");
    if let Some(actor) = actor {
        builder = builder.header(AUTHORIZATION, format!("Bearer avatar-{actor}"));
    }
    Ok(builder.body(Body::from(serde_json::to_vec(body)?))?)
}

/// Unauthenticated GET carrying the peer address the production listener would attach.
pub fn public_get(uri: &str) -> TestResult<Request<Body>> {
    let mut request = Request::get(uri).body(Body::empty())?;
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([203, 0, 113, 9], 4000))));
    Ok(request)
}

pub async fn json_body(response: Response<Body>) -> TestResult<Value> {
    let bytes = to_bytes(response.into_body(), 128 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub async fn raw_body(response: Response<Body>) -> TestResult<Vec<u8>> {
    Ok(to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await?
        .to_vec())
}

pub async fn assert_error(response: Response<Body>, status: StatusCode, code: &str) -> TestResult {
    assert_eq!(response.status(), status);
    let body = json_body(response).await?;
    assert_eq!(body["error"]["code"], code);
    assert!(body["error"]["details"].is_null());
    Uuid::try_parse(
        body["error"]["request_id"]
            .as_str()
            .ok_or("error response omitted request_id")?,
    )?;
    assert_eq!(body.as_object().map(|object| object.len()), Some(1));
    Ok(())
}

pub fn header(response: &Response<Body>, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

pub async fn insert_user(pool: &PgPool) -> TestResult<Uuid> {
    let user_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, nickname, avatar_url) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind("avatar test")
        .bind("https://images.example/original.png")
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO auth_identities (id, user_id, provider, provider_id) \
         VALUES ($1, $2, 'kakao', $3)",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(format!("avatar-{user_id}"))
    .execute(pool)
    .await?;
    Ok(user_id)
}

/// Seed one `user_avatar_uploads` row directly and return its object key.
pub async fn insert_row(
    pool: &PgPool,
    user_id: Uuid,
    upload_id: Uuid,
    status: &str,
) -> TestResult<String> {
    let key = object_key(user_id, upload_id);
    sqlx::query(
        "INSERT INTO user_avatar_uploads \
             (id, user_id, object_key, byte_size, status, expires_at, released_at) \
         VALUES ($1, $2, $3, 2048, $4, \
                 clock_timestamp() + interval '15 minutes', \
                 CASE WHEN $5 THEN clock_timestamp() ELSE NULL END)",
    )
    .bind(upload_id)
    .bind(user_id)
    .bind(&key)
    .bind(status)
    .bind(status == "released")
    .execute(pool)
    .await?;
    Ok(key)
}

pub async fn row_status(pool: &PgPool, upload_id: Uuid) -> TestResult<Option<String>> {
    Ok(
        sqlx::query_scalar::<_, String>("SELECT status FROM user_avatar_uploads WHERE id = $1")
            .bind(upload_id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn released_at_is_set(pool: &PgPool, upload_id: Uuid) -> TestResult<bool> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT released_at IS NOT NULL FROM user_avatar_uploads WHERE id = $1",
    )
    .bind(upload_id)
    .fetch_one(pool)
    .await?)
}

pub async fn count_rows(pool: &PgPool, user_id: Uuid, status: &str) -> TestResult<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM user_avatar_uploads WHERE user_id = $1 AND status = $2",
    )
    .bind(user_id)
    .bind(status)
    .fetch_one(pool)
    .await?)
}

pub async fn queued_keys(pool: &PgPool) -> TestResult<Vec<String>> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT object_key FROM account_object_deletion_intents ORDER BY object_key",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn user_avatar_url(pool: &PgPool, user_id: Uuid) -> TestResult<Option<String>> {
    Ok(
        sqlx::query_scalar::<_, Option<String>>("SELECT avatar_url FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(pool)
            .await?,
    )
}
