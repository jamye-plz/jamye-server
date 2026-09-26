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
            MediaRepository, MediaRepositoryError, MediaRepositoryFuture, PosterCandidateRecord,
            PrepareUploadFinalizeQuery, UploadFinalizePreparation, UploadFinalizeRecord,
            UploadIntentRecord,
        },
        object_storage::{
            InspectObjectRequest, MediaObjectStorage, MediaObjectStorageFuture,
            ObjectStorageProviderError, PresignPutRequest, PresignedPut,
        },
        rate_limit::{
            RateLimitError, RateLimitFuture, RateLimitOutcome, RateLimitRequest, RateLimiter,
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
    let _interest_sentinel = crate::logging_interest_sentinel();

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
                "confirmed_at": "1970-01-01T00:01:00Z",
                "poster_upload_id": null
            },
        })
    );
    Ok(())
}

#[tokio::test]
async fn md2_poster_upload_id_round_trips_and_rejects_an_unknown_poster() -> TestResult {
    let poster_upload_id = Uuid::from_u128(0xeeeeeeee_eeee_4eee_8eee_eeeeeeeeeeee);

    let accepted = harness(UploadMode::Success, FinalizeMode::ChatSuccessWithPoster)
        .oneshot(post_request(
            &format!("/api/v1/media/uploads/{UPLOAD_ID}/finalize"),
            Some(actor_id()),
            &json!({ "poster_upload_id": poster_upload_id }),
        )?)
        .await?;
    assert_eq!(accepted.status(), StatusCode::OK);
    assert_eq!(
        response_json(accepted).await?["upload"]["poster_upload_id"],
        poster_upload_id.to_string()
    );

    let omitted = harness(UploadMode::Success, FinalizeMode::ChatSuccess)
        .oneshot(post_request(
            &format!("/api/v1/media/uploads/{UPLOAD_ID}/finalize"),
            Some(actor_id()),
            &json!({}),
        )?)
        .await?;
    assert_eq!(omitted.status(), StatusCode::OK);
    assert_eq!(
        response_json(omitted).await?["upload"]["poster_upload_id"],
        Value::Null
    );

    let unknown = harness(UploadMode::Success, FinalizeMode::ChatSuccess)
        .oneshot(post_request(
            &format!("/api/v1/media/uploads/{UPLOAD_ID}/finalize"),
            Some(actor_id()),
            &json!({ "poster_upload_id": poster_upload_id }),
        )?)
        .await?;
    assert_error(
        unknown,
        StatusCode::UNPROCESSABLE_ENTITY,
        "media_poster_invalid",
    )
    .await?;
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
    ChatSuccessWithPoster,
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
        query: &'a PrepareUploadFinalizeQuery,
    ) -> MediaRepositoryFuture<'a, UploadFinalizePreparation> {
        let mode = self.finalize_mode;
        let poster_upload_id = query.poster_upload_id;
        Box::pin(async move {
            match mode {
                FinalizeMode::TargetNotAccessible => Err(MediaRepositoryError::TargetNotAccessible),
                FinalizeMode::Conflict => Err(MediaRepositoryError::FinalizeConflict),
                FinalizeMode::DatabaseUnavailable => Err(MediaRepositoryError::Unavailable),
                FinalizeMode::ChatSuccessWithPoster => Ok(UploadFinalizePreparation::Pending {
                    upload: pending_video_upload(),
                    poster: poster_upload_id.map(poster_candidate_record),
                }),
                _ => Ok(UploadFinalizePreparation::Pending {
                    upload: pending_upload(),
                    poster: None,
                }),
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
                (
                    FinalizeMode::ChatSuccess | FinalizeMode::ChatSuccessWithPoster,
                    FinalizeUploadCommand::Chat {
                        finalized,
                        poster_upload_id,
                        ..
                    },
                ) => Ok(UploadFinalizeRecord::Chat {
                    upload: confirmed_upload(finalized, poster_upload_id),
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
                FinalizeMode::ChatSuccessWithPoster => Ok(InspectedObject {
                    content_type: Some("video/mp4".to_owned()),
                    byte_size: Some(4_096),
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

fn pending_upload() -> UploadIntentRecord {
    UploadIntentRecord {
        id: upload_id(),
        user_id: actor_id(),
        scope: MediaScope::Chat,
        target_id: target_id(),
        object_key: object_key(),
        kind: MediaKind::Image,
        content_type: "image/jpeg".to_owned(),
        byte_size: 1_024,
        filename: Some(" 여름/기록.jpg ".to_owned()),
        expires_at: OffsetDateTime::UNIX_EPOCH + time::Duration::hours(1),
        created_at: OffsetDateTime::UNIX_EPOCH,
    }
}

fn confirmed_upload(
    finalized: FinalizedObject,
    poster_upload_id: Option<Uuid>,
) -> ConfirmedUploadRecord {
    ConfirmedUploadRecord {
        id: upload_id(),
        user_id: actor_id(),
        scope: MediaScope::Chat,
        target_id: target_id(),
        object_key: object_key(),
        kind: finalized.kind,
        content_type: finalized.content_type,
        byte_size: finalized.byte_size,
        duration_seconds: finalized.duration_seconds,
        filename: Some(" 여름/기록.jpg ".to_owned()),
        confirmed_at: confirmed_at(),
        poster_upload_id,
    }
}

fn pending_video_upload() -> UploadIntentRecord {
    UploadIntentRecord {
        id: upload_id(),
        user_id: actor_id(),
        scope: MediaScope::Chat,
        target_id: target_id(),
        object_key: object_key(),
        kind: MediaKind::Video,
        content_type: "video/mp4".to_owned(),
        byte_size: 4_096,
        filename: Some(" 여름/영상.mp4 ".to_owned()),
        expires_at: OffsetDateTime::UNIX_EPOCH + time::Duration::hours(1),
        created_at: OffsetDateTime::UNIX_EPOCH,
    }
}

fn poster_candidate_record(poster_upload_id: Uuid) -> PosterCandidateRecord {
    PosterCandidateRecord {
        id: poster_upload_id,
        user_id: actor_id(),
        scope: MediaScope::Chat,
        target_id: target_id(),
        kind: MediaKind::Image,
        content_type: "image/jpeg".to_owned(),
        byte_size: 2_048,
        status_confirmed: true,
        already_linked: false,
        has_own_poster: false,
    }
}

fn object_key() -> String {
    format!("chat/{TARGET_ID}/{UPLOAD_ID}")
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
