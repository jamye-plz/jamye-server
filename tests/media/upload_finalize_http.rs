use std::{any::Any, sync::Arc, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode, header::AUTHORIZATION},
};
use jamye_server::{
    application::{
        auth::{AccessIdentity, AccessTokenVerifier, AuthenticationError},
        media::{
            MediaDependencies, MediaEndpointRateLimit, MediaFinalizeDependencies,
            MediaFinalizeService, MediaService,
        },
    },
    domain::media::{FinalizedObject, InspectedObject, MediaKind, MediaScope},
    platform::logging::build_json_subscriber,
    ports::{
        media::{
            ConfirmedUploadRecord, CreateUploadIntentCommand, FinalizeUploadCommand,
            MediaRepository, MediaRepositoryError, MediaRepositoryFuture,
            PrepareUploadFinalizeQuery, TopicMediaBindingRecord, UploadFinalizePreparation,
            UploadFinalizeRecord, UploadIntentRecord,
        },
        object_storage::{
            InspectObjectRequest, MediaObjectStorage, MediaObjectStorageFuture,
            ObjectStorageProviderError, PresignPutRequest, PresignedPut,
        },
        rate_limit::{
            RateLimitError, RateLimitFuture, RateLimitOutcome, RateLimitRequest, RateLimiter,
        },
        topics::{
            CreateTopicCommand, CreateTopicOutcome, GetTopicQuery, ListTopicDatesQuery,
            ListTopicMediaQuery, ListTopicTagsQuery, ListTopicsQuery, PatchTopicCommand,
            ReplaceTopicTagsCommand, TopicDatePage, TopicMediaPage, TopicPage, TopicRecord,
            TopicStatus, TopicTagPage, TopicsRepository, TopicsRepositoryFuture,
        },
        transactions::{
            BoxTransactionHandle, TransactionFuture, TransactionHandle, TransactionManager,
        },
    },
    transport::http::media::{MediaMutationHttpState, mutation_router},
};
use serde_json::{Value, json};
use time::OffsetDateTime;
use tower::ServiceExt;
use uuid::Uuid;

use crate::{TestResult, logging_support::SharedWriter};

const TARGET_ID: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const UPLOAD_ID: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";

#[tokio::test(flavor = "current_thread")]
async fn media_resilience_structured_json_logs_exclude_presign_finalize_secrets() -> TestResult {
    let writer = SharedWriter::default();
    let output = writer.clone();
    let subscriber = build_json_subscriber(writer, "jamye_server=info")?;
    let _guard = tracing::subscriber::set_default(subscriber);

    let presign_success = harness(UploadMode::Success, FinalizeMode::ChatSuccess)
        .oneshot(post_request(
            "/api/v1/media/uploads",
            Some(actor_id()),
            &upload_payload("chat"),
        )?)
        .await?;
    assert_eq!(presign_success.status(), StatusCode::CREATED);

    let presign_failure = harness(UploadMode::StorageUnavailable, FinalizeMode::ChatSuccess)
        .oneshot(post_request(
            "/api/v1/media/uploads",
            Some(actor_id()),
            &upload_payload("chat"),
        )?)
        .await?;
    assert_error(
        presign_failure,
        StatusCode::SERVICE_UNAVAILABLE,
        "object_storage_degraded",
    )
    .await?;

    let finalize_success = harness(UploadMode::Success, FinalizeMode::ChatSuccess)
        .oneshot(post_request(
            &format!("/api/v1/media/uploads/{UPLOAD_ID}/finalize"),
            Some(actor_id()),
            &json!({}),
        )?)
        .await?;
    assert_eq!(finalize_success.status(), StatusCode::OK);

    let finalize_failure = harness(UploadMode::Success, FinalizeMode::StorageUnavailable)
        .oneshot(post_request(
            &format!("/api/v1/media/uploads/{UPLOAD_ID}/finalize"),
            Some(actor_id()),
            &json!({}),
        )?)
        .await?;
    assert_error(
        finalize_failure,
        StatusCode::SERVICE_UNAVAILABLE,
        "object_storage_degraded",
    )
    .await?;

    let logs = output.snapshot()?;
    let entries = output.parsed_lines()?;
    assert!(!entries.is_empty());
    for entry in &entries {
        assert!(
            entry.is_object(),
            "structured media log was not a JSON object"
        );
    }
    for expected in [
        "media_upload_intent_created",
        "media_upload_finalized",
        "object_storage_degraded",
        "request_id",
    ] {
        assert!(logs.contains(expected), "logs omitted {expected}");
    }
    for forbidden in [
        "https://media.example.test/",
        "X-Amz-Signature=test",
        TARGET_ID,
        UPLOAD_ID,
        "TASK8_MINIO_ACCESS_KEY_SENTINEL",
        "TASK8_MINIO_SECRET_KEY_SENTINEL",
        "Authorization: AWS4-HMAC-SHA256",
    ] {
        assert!(!logs.contains(forbidden), "logs leaked {forbidden}");
    }
    Ok(())
}

#[tokio::test]
async fn mutation_routes_require_bearer_auth_and_reject_malformed_inputs() -> TestResult {
    let router = harness(UploadMode::Success, FinalizeMode::ChatSuccess);

    let unauthorized = router
        .clone()
        .oneshot(post_request(
            "/api/v1/media/uploads",
            None,
            &upload_payload("chat"),
        )?)
        .await?;
    assert_error(
        unauthorized,
        StatusCode::UNAUTHORIZED,
        "authentication_required",
    )
    .await?;

    let malformed_intent = router
        .clone()
        .oneshot(post_request(
            "/api/v1/media/uploads",
            Some(actor_id()),
            &upload_payload("other"),
        )?)
        .await?;
    assert_error(
        malformed_intent,
        StatusCode::UNPROCESSABLE_ENTITY,
        "request_validation_failed",
    )
    .await?;

    let malformed_finalize = router
        .oneshot(post_request(
            "/api/v1/media/uploads/not-a-uuid/finalize",
            Some(actor_id()),
            &json!({"width": 0, "height": 600}),
        )?)
        .await?;
    assert_error(
        malformed_finalize,
        StatusCode::UNPROCESSABLE_ENTITY,
        "request_validation_failed",
    )
    .await
}

#[tokio::test]
async fn md1_returns_a_server_minted_intent_and_constrained_put() -> TestResult {
    let response = harness(UploadMode::Success, FinalizeMode::ChatSuccess)
        .oneshot(post_request(
            "/api/v1/media/uploads",
            Some(actor_id()),
            &json!({
                "scope": "chat",
                "target_id": TARGET_ID,
                "content_type": "audio/mp4",
                "byte_size": 1024,
                "filename": " 음성/메모.m4a "
            }),
        )?)
        .await?;

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = response_json(response).await?;
    let upload_id = Uuid::try_parse(
        body["upload"]["id"]
            .as_str()
            .ok_or("upload response omitted id")?,
    )?;
    let object_key = format!("chat/{TARGET_ID}/{upload_id}");
    let signed_url = format!("https://media.example.test/{object_key}?X-Amz-Signature=test");
    assert_eq!(
        body,
        json!({
            "upload": {
                "id": upload_id,
                "scope": "chat",
                "target_id": TARGET_ID,
                "object_key": object_key,
                "kind": "audio",
                "content_type": "audio/mp4",
                "byte_size": 1024,
                "filename": " 음성/메모.m4a ",
                "expires_at": "1970-01-01T01:00:00Z",
                "created_at": "1970-01-01T00:00:00Z"
            },
            "put": {
                "url": signed_url,
                "expires_in": 3600
            }
        })
    );
    Ok(())
}

#[tokio::test]
async fn md1_maps_rate_limit_and_dependency_failures_to_stable_envelopes() -> TestResult {
    let rate_limited = harness(UploadMode::RateLimited, FinalizeMode::ChatSuccess)
        .oneshot(post_request(
            "/api/v1/media/uploads",
            Some(actor_id()),
            &upload_payload("chat"),
        )?)
        .await?;
    assert_eq!(
        rate_limited
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok()),
        Some("7")
    );
    assert_error(
        rate_limited,
        StatusCode::TOO_MANY_REQUESTS,
        "rate_limit_exceeded",
    )
    .await?;

    for (mode, code) in [
        (UploadMode::RateLimitUnavailable, "rate_limit_unavailable"),
        (UploadMode::TargetNotAccessible, "media_not_accessible"),
        (UploadMode::DatabaseUnavailable, "database_unavailable"),
        (UploadMode::StorageUnavailable, "object_storage_degraded"),
    ] {
        let status = if matches!(mode, UploadMode::TargetNotAccessible) {
            StatusCode::FORBIDDEN
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        };
        let response = harness(mode, FinalizeMode::ChatSuccess)
            .oneshot(post_request(
                "/api/v1/media/uploads",
                Some(actor_id()),
                &upload_payload("chat"),
            )?)
            .await?;
        assert_error(response, status, code).await?;
    }
    Ok(())
}

#[tokio::test]
async fn md2_chat_finalize_returns_the_confirmed_unbound_capability() -> TestResult {
    let response = harness(UploadMode::Success, FinalizeMode::ChatSuccess)
        .oneshot(post_request(
            &format!("/api/v1/media/uploads/{UPLOAD_ID}/finalize"),
            Some(actor_id()),
            &json!({}),
        )?)
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await?,
        json!({
            "scope": "chat",
            "status": "confirmed",
            "bound": false,
            "upload": {
                "id": UPLOAD_ID,
                "scope": "chat",
                "target_id": TARGET_ID,
                "object_key": format!("chat/{TARGET_ID}/{UPLOAD_ID}"),
                "kind": "image",
                "content_type": "image/jpeg",
                "byte_size": 1024,
                "duration": null,
                "filename": " 여름/기록.jpg ",
                "confirmed_at": "1970-01-01T00:01:00Z"
            },
            "topic_media": null,
            "topic_status": null
        })
    );
    Ok(())
}

#[tokio::test]
async fn md2_topic_finalize_returns_bound_media_and_enriched_status() -> TestResult {
    let response = harness(UploadMode::Success, FinalizeMode::TopicSuccess)
        .oneshot(post_request(
            &format!("/api/v1/media/uploads/{UPLOAD_ID}/finalize"),
            Some(actor_id()),
            &json!({"width": 800, "height": 600}),
        )?)
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await?;
    Uuid::try_parse(
        body["topic_media"]["id"]
            .as_str()
            .ok_or("topic finalize response omitted media id")?,
    )?;
    assert_eq!(body["scope"], "topic");
    assert_eq!(body["status"], "bound");
    assert_eq!(body["bound"], true);
    assert_eq!(body["upload"]["id"], UPLOAD_ID);
    assert_eq!(body["upload"]["scope"], "topic");
    assert_eq!(body["upload"]["target_id"], TARGET_ID);
    assert_eq!(body["topic_media"]["topic_id"], TARGET_ID);
    assert_eq!(body["topic_media"]["media_upload_id"], UPLOAD_ID);
    assert_eq!(body["topic_media"]["content_type"], "image/jpeg");
    assert_eq!(body["topic_media"]["width"], 800);
    assert_eq!(body["topic_media"]["height"], 600);
    assert_eq!(body["topic_media"]["byte_size"], 1024);
    assert_eq!(body["topic_status"], "enriched");
    assert_eq!(body.as_object().map(|object| object.len()), Some(6));
    Ok(())
}

#[tokio::test]
async fn md2_maps_conflict_bola_validation_and_dependencies_to_stable_envelopes() -> TestResult {
    for (mode, status, code) in [
        (
            FinalizeMode::TargetNotAccessible,
            StatusCode::FORBIDDEN,
            "media_not_accessible",
        ),
        (
            FinalizeMode::Conflict,
            StatusCode::CONFLICT,
            "media_finalize_conflict",
        ),
        (
            FinalizeMode::Validation,
            StatusCode::UNPROCESSABLE_ENTITY,
            "media_finalize_validation_failed",
        ),
        (
            FinalizeMode::StorageUnavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            "object_storage_degraded",
        ),
        (
            FinalizeMode::DatabaseUnavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        ),
    ] {
        let response = harness(UploadMode::Success, mode)
            .oneshot(post_request(
                &format!("/api/v1/media/uploads/{UPLOAD_ID}/finalize"),
                Some(actor_id()),
                &json!({}),
            )?)
            .await?;
        assert_error(response, status, code).await?;
    }
    Ok(())
}

fn harness(upload_mode: UploadMode, finalize_mode: FinalizeMode) -> Router {
    let transactions = Arc::new(FakeTransactions);
    let repository = Arc::new(FakeRepository {
        upload_mode,
        finalize_mode,
    });
    let object_storage = Arc::new(FakeObjectStorage {
        upload_mode,
        finalize_mode,
    });
    let Ok(uploads) = MediaService::new(
        MediaDependencies {
            transactions: transactions.clone(),
            repository: repository.clone(),
            object_storage: object_storage.clone(),
            rate_limiter: Arc::new(FakeRateLimiter(upload_mode)),
        },
        MediaEndpointRateLimit {
            limit: 2,
            window: Duration::from_secs(60),
        },
    ) else {
        panic!("test upload configuration must be valid");
    };
    let uploads = Arc::new(uploads);
    let finalize = Arc::new(MediaFinalizeService::new(MediaFinalizeDependencies {
        transactions,
        repository,
        object_storage,
        topics: Arc::new(FakeTopicsRepository),
    }));
    mutation_router(MediaMutationHttpState::new(
        uploads,
        finalize,
        Arc::new(TestAccessVerifier),
    ))
}

fn upload_payload(scope: &str) -> Value {
    json!({
        "scope": scope,
        "target_id": TARGET_ID,
        "content_type": "image/jpeg",
        "byte_size": 1024,
        "filename": null
    })
}

fn post_request(uri: &str, actor_id: Option<Uuid>, payload: &Value) -> TestResult<Request<Body>> {
    let mut request = Request::post(uri).header("content-type", "application/json");
    if let Some(actor_id) = actor_id {
        request = request.header(AUTHORIZATION, format!("Bearer task8-{actor_id}"));
    }
    Ok(request.body(Body::from(serde_json::to_vec(payload)?))?)
}

async fn assert_error(response: Response<Body>, status: StatusCode, code: &str) -> TestResult {
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
    Ok(())
}

async fn response_json(response: Response<Body>) -> TestResult<Value> {
    let bytes = to_bytes(response.into_body(), 128 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[derive(Clone, Copy)]
enum UploadMode {
    Success,
    RateLimited,
    RateLimitUnavailable,
    TargetNotAccessible,
    DatabaseUnavailable,
    StorageUnavailable,
}

#[derive(Clone, Copy)]
enum FinalizeMode {
    ChatSuccess,
    TopicSuccess,
    TargetNotAccessible,
    Conflict,
    Validation,
    StorageUnavailable,
    DatabaseUnavailable,
}

struct FakeHandle;

impl TransactionHandle for FakeHandle {
    fn as_any_mut(&mut self) -> &mut (dyn Any + Send) {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }
}

struct FakeTransactions;

impl TransactionManager for FakeTransactions {
    fn begin(&self) -> TransactionFuture<'_, BoxTransactionHandle> {
        Box::pin(async { Ok(Box::new(FakeHandle) as BoxTransactionHandle) })
    }

    fn commit<'a>(&'a self, _handle: BoxTransactionHandle) -> TransactionFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn rollback<'a>(&'a self, _handle: BoxTransactionHandle) -> TransactionFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

struct FakeRateLimiter(UploadMode);

impl RateLimiter for FakeRateLimiter {
    fn check<'a>(&'a self, _request: &'a RateLimitRequest) -> RateLimitFuture<'a> {
        let mode = self.0;
        Box::pin(async move {
            match mode {
                UploadMode::RateLimited => Ok(RateLimitOutcome::Denied {
                    retry_after: Duration::from_secs(7),
                }),
                UploadMode::RateLimitUnavailable => Err(RateLimitError),
                _ => Ok(RateLimitOutcome::Allowed),
            }
        })
    }
}

struct FakeRepository {
    upload_mode: UploadMode,
    finalize_mode: FinalizeMode,
}

impl MediaRepository for FakeRepository {
    fn create_upload_intent<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        command: &'a CreateUploadIntentCommand,
    ) -> MediaRepositoryFuture<'a, UploadIntentRecord> {
        let mode = self.upload_mode;
        let command = command.clone();
        Box::pin(async move {
            match mode {
                UploadMode::TargetNotAccessible => {
                    return Err(MediaRepositoryError::TargetNotAccessible);
                }
                UploadMode::DatabaseUnavailable => {
                    return Err(MediaRepositoryError::Unavailable);
                }
                _ => {}
            }
            let created_at = OffsetDateTime::UNIX_EPOCH;
            let expires_in = i64::try_from(command.expires_in.as_secs())
                .map_err(|_| MediaRepositoryError::InvalidData)?;
            let expires_at = created_at + time::Duration::seconds(expires_in);
            Ok(UploadIntentRecord {
                id: command.id,
                user_id: command.user_id,
                scope: command.scope,
                target_id: command.target_id,
                object_key: command.object_key,
                kind: command.kind,
                content_type: command.content_type,
                byte_size: command.byte_size,
                filename: command.filename,
                expires_at,
                created_at,
            })
        })
    }

    fn prepare_upload_finalize<'a>(
        &'a self,
        _query: &'a PrepareUploadFinalizeQuery,
    ) -> MediaRepositoryFuture<'a, UploadFinalizePreparation> {
        let mode = self.finalize_mode;
        Box::pin(async move {
            match mode {
                FinalizeMode::TargetNotAccessible => Err(MediaRepositoryError::TargetNotAccessible),
                FinalizeMode::Conflict => Err(MediaRepositoryError::FinalizeConflict),
                FinalizeMode::DatabaseUnavailable => Err(MediaRepositoryError::Unavailable),
                _ => Ok(UploadFinalizePreparation::Pending(pending_upload(
                    finalize_scope(mode),
                ))),
            }
        })
    }

    fn finalize_upload<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        command: &'a FinalizeUploadCommand,
    ) -> MediaRepositoryFuture<'a, UploadFinalizeRecord> {
        let mode = self.finalize_mode;
        let command = command.clone();
        Box::pin(async move {
            match (mode, command) {
                (FinalizeMode::ChatSuccess, FinalizeUploadCommand::Chat { finalized, .. }) => {
                    Ok(UploadFinalizeRecord::Chat {
                        upload: confirmed_upload(MediaScope::Chat, finalized),
                    })
                }
                (
                    FinalizeMode::TopicSuccess,
                    FinalizeUploadCommand::Topic {
                        topic_media_id,
                        width,
                        height,
                        finalized,
                        ..
                    },
                ) => Ok(UploadFinalizeRecord::Topic {
                    upload: confirmed_upload(MediaScope::Topic, finalized.clone()),
                    topic_media: TopicMediaBindingRecord {
                        id: topic_media_id,
                        topic_id: target_id(),
                        media_upload_id: upload_id(),
                        object_key: object_key(MediaScope::Topic),
                        content_type: finalized.content_type,
                        width,
                        height,
                        byte_size: finalized.byte_size,
                        created_at: confirmed_at(),
                    },
                }),
                _ => Err(MediaRepositoryError::InvalidData),
            }
        })
    }
}

struct FakeObjectStorage {
    upload_mode: UploadMode,
    finalize_mode: FinalizeMode,
}

impl MediaObjectStorage for FakeObjectStorage {
    fn presign_put<'a>(
        &'a self,
        request: &'a PresignPutRequest,
    ) -> MediaObjectStorageFuture<'a, PresignedPut> {
        let mode = self.upload_mode;
        let request = request.clone();
        Box::pin(async move {
            if matches!(mode, UploadMode::StorageUnavailable) {
                return Err(ObjectStorageProviderError::Unavailable);
            }
            Ok(PresignedPut {
                url: format!(
                    "https://media.example.test/{}?X-Amz-Signature=test",
                    request.object_key
                ),
                expires_in: request.expires_in,
            })
        })
    }

    fn inspect_object<'a>(
        &'a self,
        _request: &'a InspectObjectRequest,
    ) -> MediaObjectStorageFuture<'a, InspectedObject> {
        let mode = self.finalize_mode;
        Box::pin(async move {
            match mode {
                FinalizeMode::StorageUnavailable => Err(ObjectStorageProviderError::Unavailable),
                FinalizeMode::Validation => Ok(InspectedObject {
                    content_type: Some("image/png".to_owned()),
                    byte_size: Some(1_024),
                    audio_duration: None,
                }),
                _ => Ok(InspectedObject {
                    content_type: Some("image/jpeg".to_owned()),
                    byte_size: Some(1_024),
                    audio_duration: None,
                }),
            }
        })
    }
}

struct FakeTopicsRepository;

impl TopicsRepository for FakeTopicsRepository {
    fn create_topic<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _command: &'a CreateTopicCommand,
    ) -> TopicsRepositoryFuture<'a, CreateTopicOutcome> {
        Box::pin(async { panic!("media HTTP must not create topics") })
    }

    fn patch_topic<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _command: &'a PatchTopicCommand,
    ) -> TopicsRepositoryFuture<'a, TopicRecord> {
        Box::pin(async { panic!("media HTTP must not patch topics") })
    }

    fn promote_enriched<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _topic_id: Uuid,
    ) -> TopicsRepositoryFuture<'a, TopicStatus> {
        Box::pin(async { Ok(TopicStatus::Enriched) })
    }

    fn replace_tags<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _command: &'a ReplaceTopicTagsCommand,
    ) -> TopicsRepositoryFuture<'a, TopicTagPage> {
        Box::pin(async { panic!("media HTTP must not replace tags") })
    }

    fn list_topics(&self, _query: ListTopicsQuery) -> TopicsRepositoryFuture<'_, TopicPage> {
        Box::pin(async { panic!("media HTTP must not list topics") })
    }

    fn list_topic_dates(
        &self,
        _query: ListTopicDatesQuery,
    ) -> TopicsRepositoryFuture<'_, TopicDatePage> {
        Box::pin(async { panic!("media HTTP must not list topic dates") })
    }

    fn get_topic(&self, _query: GetTopicQuery) -> TopicsRepositoryFuture<'_, TopicRecord> {
        Box::pin(async { panic!("media HTTP must not get topics") })
    }

    fn list_tags(&self, _query: ListTopicTagsQuery) -> TopicsRepositoryFuture<'_, TopicTagPage> {
        Box::pin(async { panic!("media HTTP must not list tags") })
    }

    fn list_media(
        &self,
        _query: ListTopicMediaQuery,
    ) -> TopicsRepositoryFuture<'_, TopicMediaPage> {
        Box::pin(async { panic!("media HTTP must not list topic media") })
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct TestAccessVerifier;

impl AccessTokenVerifier for TestAccessVerifier {
    fn verify(&self, token: &str) -> Result<AccessIdentity, AuthenticationError> {
        let actor_id = token
            .strip_prefix("task8-")
            .and_then(|value| Uuid::try_parse(value).ok())
            .ok_or(AuthenticationError)?;
        Ok(AccessIdentity::new(actor_id, Uuid::nil(), "task-8-test"))
    }
}

fn finalize_scope(mode: FinalizeMode) -> MediaScope {
    if matches!(mode, FinalizeMode::TopicSuccess) {
        MediaScope::Topic
    } else {
        MediaScope::Chat
    }
}

fn pending_upload(scope: MediaScope) -> UploadIntentRecord {
    UploadIntentRecord {
        id: upload_id(),
        user_id: actor_id(),
        scope,
        target_id: target_id(),
        object_key: object_key(scope),
        kind: MediaKind::Image,
        content_type: "image/jpeg".to_owned(),
        byte_size: 1_024,
        filename: Some(" 여름/기록.jpg ".to_owned()),
        expires_at: OffsetDateTime::UNIX_EPOCH + time::Duration::hours(1),
        created_at: OffsetDateTime::UNIX_EPOCH,
    }
}

fn confirmed_upload(scope: MediaScope, finalized: FinalizedObject) -> ConfirmedUploadRecord {
    ConfirmedUploadRecord {
        id: upload_id(),
        user_id: actor_id(),
        scope,
        target_id: target_id(),
        object_key: object_key(scope),
        kind: finalized.kind,
        content_type: finalized.content_type,
        byte_size: finalized.byte_size,
        duration_seconds: finalized.duration_seconds,
        filename: Some(" 여름/기록.jpg ".to_owned()),
        confirmed_at: confirmed_at(),
    }
}

fn object_key(scope: MediaScope) -> String {
    let prefix = match scope {
        MediaScope::Chat => "chat",
        MediaScope::Topic => "topics",
    };
    format!("{prefix}/{TARGET_ID}/{UPLOAD_ID}")
}

fn confirmed_at() -> OffsetDateTime {
    OffsetDateTime::UNIX_EPOCH + time::Duration::minutes(1)
}

fn actor_id() -> Uuid {
    Uuid::from_u128(0xaaaaaaaa_aaaa_4aaa_8aaa_aaaaaaaaaaaa)
}

fn target_id() -> Uuid {
    Uuid::from_u128(0xbbbbbbbb_bbbb_4bbb_8bbb_bbbbbbbbbbbb)
}

fn upload_id() -> Uuid {
    Uuid::from_u128(0xcccccccc_cccc_4ccc_8ccc_cccccccccccc)
}
