use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode, header::AUTHORIZATION},
};
use jamye_server::{
    application::{
        auth::{AccessIdentity, AccessTokenVerifier, AuthenticationError},
        media::{MediaAccessDependencies, MediaAccessService},
    },
    domain::media::InspectedObject,
    platform::logging::build_json_subscriber,
    ports::{
        media::{
            AuthorizeMediaAccessQuery, CreateUploadIntentCommand, FinalizeUploadCommand,
            MediaAccessRecord, MediaRepository, MediaRepositoryError, MediaRepositoryFuture,
            PrepareUploadFinalizeQuery, UploadFinalizePreparation, UploadFinalizeRecord,
            UploadIntentRecord,
        },
        object_storage::{
            InspectObjectRequest, MediaObjectStorage, MediaObjectStorageFuture,
            ObjectStorageProviderError, PresignGetRequest, PresignPutRequest, PresignedGet,
            PresignedPut,
        },
        transactions::TransactionHandle,
    },
    transport::http::media::{MediaHttpState, router as media_router},
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use crate::{TestResult, logging_support::SharedWriter};

const MEDIA_ID: &str = "11111111-1111-4111-8111-111111111111";
const UPLOAD_ID: &str = "22222222-2222-4222-8222-222222222222";
const OBJECT_KEY: &str =
    "chat/33333333-3333-4333-8333-333333333333/22222222-2222-4222-8222-222222222222";
const SIGNED_URL: &str = concat!(
    "https://media.example.test/chat/33333333-3333-4333-8333-333333333333/",
    "22222222-2222-4222-8222-222222222222?X-Amz-Signature=test"
);

#[tokio::test(flavor = "current_thread")]
async fn media_resilience_structured_json_logs_exclude_access_secrets() -> TestResult {
    let writer = SharedWriter::default();
    let output = writer.clone();
    let subscriber = build_json_subscriber(writer, "jamye_server=info")?;
    let _guard = tracing::subscriber::set_default(subscriber);
    let _interest_sentinel = crate::logging_interest_sentinel();

    let view_success = harness(RepositoryMode::Success, StorageMode::Success)
        .oneshot(request(
            &format!("/api/v1/media/{MEDIA_ID}/url"),
            Some(actor_id()),
        )?)
        .await?;
    assert_eq!(view_success.status(), StatusCode::OK);

    let download_success = harness(RepositoryMode::Success, StorageMode::Success)
        .oneshot(request(
            &format!("/api/v1/media/{MEDIA_ID}/download"),
            Some(actor_id()),
        )?)
        .await?;
    assert_eq!(download_success.status(), StatusCode::TEMPORARY_REDIRECT);

    for path in ["url", "download"] {
        let failure = harness(RepositoryMode::Success, StorageMode::Unavailable)
            .oneshot(request(
                &format!("/api/v1/media/{MEDIA_ID}/{path}"),
                Some(actor_id()),
            )?)
            .await?;
        assert_error(
            failure,
            StatusCode::SERVICE_UNAVAILABLE,
            "object_storage_degraded",
        )
        .await?;
    }

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
        "media_view_url_issued",
        "media_download_url_issued",
        "object_storage_degraded",
        "request_id",
    ] {
        assert!(logs.contains(expected), "logs omitted {expected}");
    }
    for forbidden in [
        SIGNED_URL,
        OBJECT_KEY,
        MEDIA_ID,
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
async fn access_routes_require_bearer_auth_and_reject_malformed_media_ids() -> TestResult {
    let router = harness(RepositoryMode::Success, StorageMode::Success);

    let unauthorized = router
        .clone()
        .oneshot(request(&format!("/api/v1/media/{MEDIA_ID}/url"), None)?)
        .await?;
    assert_error(
        unauthorized,
        StatusCode::UNAUTHORIZED,
        "authentication_required",
    )
    .await?;

    let malformed = router
        .oneshot(request("/api/v1/media/not-a-uuid/url", Some(actor_id()))?)
        .await?;
    assert_error(
        malformed,
        StatusCode::UNPROCESSABLE_ENTITY,
        "request_validation_failed",
    )
    .await
}

#[tokio::test]
async fn md4_returns_only_public_metadata_and_the_short_reissued_url() -> TestResult {
    let response = harness(RepositoryMode::Success, StorageMode::Success)
        .oneshot(request(
            &format!("/api/v1/media/{MEDIA_ID}/url"),
            Some(actor_id()),
        )?)
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await?,
        json!({
            "id": MEDIA_ID,
            "media_upload_id": UPLOAD_ID,
            "url": SIGNED_URL,
            "content_type": "image/jpeg",
            "byte_size": 1024,
            "width": 800,
            "height": 600,
            "duration": null,
            "filename": "여름 기록 (최종).jpg",
            "expires_in": 600
        })
    );
    Ok(())
}

#[tokio::test]
async fn md5_returns_an_empty_307_redirect_to_the_authorized_download_url() -> TestResult {
    let response = harness(RepositoryMode::Success, StorageMode::Success)
        .oneshot(request(
            &format!("/api/v1/media/{MEDIA_ID}/download"),
            Some(actor_id()),
        )?)
        .await?;

    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some(SIGNED_URL)
    );
    assert!(to_bytes(response.into_body(), 1024).await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn missing_or_cross_group_media_uses_one_non_disclosing_bola_envelope() -> TestResult {
    let response = harness(RepositoryMode::TargetNotAccessible, StorageMode::Success)
        .oneshot(request(
            &format!("/api/v1/media/{MEDIA_ID}/url"),
            Some(actor_id()),
        )?)
        .await?;

    assert_error(response, StatusCode::FORBIDDEN, "media_not_accessible").await
}

#[tokio::test]
async fn access_maps_storage_and_database_failures_to_only_their_stable_503_codes() -> TestResult {
    let storage = harness(RepositoryMode::Success, StorageMode::Unavailable)
        .oneshot(request(
            &format!("/api/v1/media/{MEDIA_ID}/download"),
            Some(actor_id()),
        )?)
        .await?;
    assert_error(
        storage,
        StatusCode::SERVICE_UNAVAILABLE,
        "object_storage_degraded",
    )
    .await?;

    let database = harness(RepositoryMode::Unavailable, StorageMode::Success)
        .oneshot(request(
            &format!("/api/v1/media/{MEDIA_ID}/url"),
            Some(actor_id()),
        )?)
        .await?;
    assert_error(
        database,
        StatusCode::SERVICE_UNAVAILABLE,
        "database_unavailable",
    )
    .await
}

fn harness(repository_mode: RepositoryMode, storage_mode: StorageMode) -> Router {
    let service = Arc::new(MediaAccessService::new(MediaAccessDependencies {
        repository: Arc::new(FakeRepository(repository_mode)),
        object_storage: Arc::new(FakeObjectStorage(storage_mode)),
    }));
    media_router(MediaHttpState::new(service, Arc::new(TestAccessVerifier)))
}

fn request(uri: &str, actor_id: Option<Uuid>) -> TestResult<Request<Body>> {
    let mut request = Request::get(uri);
    if let Some(actor_id) = actor_id {
        request = request.header(AUTHORIZATION, format!("Bearer task8-{actor_id}"));
    }
    Ok(request.body(Body::empty())?)
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
enum RepositoryMode {
    Success,
    TargetNotAccessible,
    Unavailable,
}

struct FakeRepository(RepositoryMode);

impl MediaRepository for FakeRepository {
    fn create_upload_intent<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _command: &'a CreateUploadIntentCommand,
    ) -> MediaRepositoryFuture<'a, UploadIntentRecord> {
        Box::pin(async { panic!("access HTTP must not create upload intents") })
    }

    fn prepare_upload_finalize<'a>(
        &'a self,
        _query: &'a PrepareUploadFinalizeQuery,
    ) -> MediaRepositoryFuture<'a, UploadFinalizePreparation> {
        Box::pin(async { panic!("access HTTP must not prepare finalize") })
    }

    fn finalize_upload<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _command: &'a FinalizeUploadCommand,
    ) -> MediaRepositoryFuture<'a, UploadFinalizeRecord> {
        Box::pin(async { panic!("access HTTP must not finalize uploads") })
    }

    fn authorize_media_access<'a>(
        &'a self,
        _query: &'a AuthorizeMediaAccessQuery,
    ) -> MediaRepositoryFuture<'a, MediaAccessRecord> {
        let mode = self.0;
        Box::pin(async move {
            match mode {
                RepositoryMode::Success => Ok(media_record()),
                RepositoryMode::TargetNotAccessible => {
                    Err(MediaRepositoryError::TargetNotAccessible)
                }
                RepositoryMode::Unavailable => Err(MediaRepositoryError::Unavailable),
            }
        })
    }
}

#[derive(Clone, Copy)]
enum StorageMode {
    Success,
    Unavailable,
}

struct FakeObjectStorage(StorageMode);

impl MediaObjectStorage for FakeObjectStorage {
    fn presign_put<'a>(
        &'a self,
        _request: &'a PresignPutRequest,
    ) -> MediaObjectStorageFuture<'a, PresignedPut> {
        Box::pin(async { panic!("access HTTP must not presign PUT") })
    }

    fn inspect_object<'a>(
        &'a self,
        _request: &'a InspectObjectRequest,
    ) -> MediaObjectStorageFuture<'a, InspectedObject> {
        Box::pin(async { panic!("access HTTP must not inspect objects") })
    }

    fn presign_get<'a>(
        &'a self,
        _request: &'a PresignGetRequest,
    ) -> MediaObjectStorageFuture<'a, PresignedGet> {
        let mode = self.0;
        Box::pin(async move {
            match mode {
                StorageMode::Success => Ok(PresignedGet {
                    url: SIGNED_URL.to_owned(),
                    expires_in: Duration::from_secs(600),
                }),
                StorageMode::Unavailable => Err(ObjectStorageProviderError::Unavailable),
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

fn media_record() -> MediaAccessRecord {
    MediaAccessRecord {
        id: media_id(),
        media_upload_id: upload_id(),
        object_key: OBJECT_KEY.to_owned(),
        content_type: "image/jpeg".to_owned(),
        byte_size: 1024,
        width: Some(800),
        height: Some(600),
        duration_seconds: None,
        filename: Some("여름 기록 (최종).jpg".to_owned()),
    }
}

fn actor_id() -> Uuid {
    Uuid::from_u128(0xaaaaaaaa_aaaa_4aaa_8aaa_aaaaaaaaaaaa)
}

fn media_id() -> Uuid {
    Uuid::from_u128(0x11111111_1111_4111_8111_111111111111)
}

fn upload_id() -> Uuid {
    Uuid::from_u128(0x22222222_2222_4222_8222_222222222222)
}
