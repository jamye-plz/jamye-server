//! Authenticated HTTP boundary for private-media access.

use std::sync::Arc;

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{FromRef, Path, Request, State},
    http::{
        HeaderValue, StatusCode,
        header::{LOCATION, RETRY_AFTER},
    },
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    application::{
        auth::AccessTokenVerifier,
        media::{
            MediaAccessService, MediaAccessUrl, MediaError, MediaFinalizeService, MediaService,
            UploadFinalizeInput, UploadFinalizeResult, UploadIntentCreateInput,
            UploadIntentWithPresignedPut,
        },
    },
    domain::media::{MediaKind, MediaScope},
    ports::{
        media::{ConfirmedUploadRecord, TopicMediaBindingRecord, UploadIntentRecord},
        object_storage::PresignedPut,
    },
    transport::http::auth::{AuthVerifierState, AuthenticatedAccess, error_response, request_id},
};

const MAX_REQUEST_BYTES: usize = 64 * 1024;

#[derive(Clone)]
pub struct MediaHttpState {
    access: Arc<MediaAccessService>,
    verifier: AuthVerifierState,
}

#[derive(Clone)]
pub struct MediaMutationHttpState {
    uploads: Arc<MediaService>,
    finalize: Arc<MediaFinalizeService>,
    verifier: AuthVerifierState,
}

impl MediaHttpState {
    pub fn new(access: Arc<MediaAccessService>, verifier: Arc<dyn AccessTokenVerifier>) -> Self {
        Self {
            access,
            verifier: AuthVerifierState::new(verifier),
        }
    }
}

impl MediaMutationHttpState {
    pub fn new(
        uploads: Arc<MediaService>,
        finalize: Arc<MediaFinalizeService>,
        verifier: Arc<dyn AccessTokenVerifier>,
    ) -> Self {
        Self {
            uploads,
            finalize,
            verifier: AuthVerifierState::new(verifier),
        }
    }
}

impl FromRef<MediaHttpState> for AuthVerifierState {
    fn from_ref(state: &MediaHttpState) -> Self {
        state.verifier.clone()
    }
}

impl FromRef<MediaMutationHttpState> for AuthVerifierState {
    fn from_ref(state: &MediaMutationHttpState) -> Self {
        state.verifier.clone()
    }
}

pub fn router(state: MediaHttpState) -> Router {
    Router::new()
        .route("/api/v1/media/{media_id}/url", get(view_url))
        .route("/api/v1/media/{media_id}/download", get(download))
        .with_state(state)
}

pub fn mutation_router(state: MediaMutationHttpState) -> Router {
    Router::new()
        .route("/api/v1/media/uploads", post(create_upload_intent))
        .route(
            "/api/v1/media/uploads/{upload_id}/finalize",
            post(finalize_upload),
        )
        .with_state(state)
}

async fn create_upload_intent(
    State(state): State<MediaMutationHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    let result = match parse_upload_intent_request(body).await {
        Ok(input) => {
            state
                .uploads
                .create_upload_intent(identity.user_id, input)
                .await
        }
        Err(error) => Err(error),
    };

    match result {
        Ok(intent) => {
            tracing::info!(
                request_id = %request_id,
                event_code = "media_upload_intent_created",
                "media request completed"
            );
            (
                StatusCode::CREATED,
                Json(UploadIntentResponse::from(intent)),
            )
                .into_response()
        }
        Err(error) => MediaHttpError { error, request_id }.into_response(),
    }
}

async fn finalize_upload(
    State(state): State<MediaMutationHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(upload_id): Path<String>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    let result = match parse_finalize_request(&upload_id, body).await {
        Ok((upload_id, input)) => {
            state
                .finalize
                .finalize_upload(identity.user_id, upload_id, input)
                .await
        }
        Err(error) => Err(error),
    };

    match result {
        Ok(finalized) => {
            tracing::info!(
                request_id = %request_id,
                event_code = "media_upload_finalized",
                "media request completed"
            );
            (
                StatusCode::OK,
                Json(UploadFinalizeResponse::from(finalized)),
            )
                .into_response()
        }
        Err(error) => MediaHttpError { error, request_id }.into_response(),
    }
}

async fn parse_upload_intent_request(body: Body) -> Result<UploadIntentCreateInput, MediaError> {
    let payload = parse_json::<UploadIntentCreateRequest>(body).await?;
    Ok(UploadIntentCreateInput {
        scope: payload.scope.into(),
        target_id: payload.target_id,
        content_type: payload.content_type,
        byte_size: payload.byte_size,
        filename: payload.filename,
    })
}

async fn parse_finalize_request(
    upload_id: &str,
    body: Body,
) -> Result<(Uuid, UploadFinalizeInput), MediaError> {
    let upload_id = Uuid::try_parse(upload_id).map_err(|_| MediaError::RequestValidation)?;
    let payload = parse_json::<UploadFinalizeRequest>(body).await?;
    if payload.width == Some(0) || payload.height == Some(0) {
        return Err(MediaError::RequestValidation);
    }
    Ok((
        upload_id,
        UploadFinalizeInput {
            width: payload.width,
            height: payload.height,
        },
    ))
}

async fn parse_json<T>(body: Body) -> Result<T, MediaError>
where
    T: DeserializeOwned,
{
    let bytes = to_bytes(body, MAX_REQUEST_BYTES)
        .await
        .map_err(|_| MediaError::RequestValidation)?;
    serde_json::from_slice(&bytes).map_err(|_| MediaError::RequestValidation)
}

async fn view_url(
    State(state): State<MediaHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(media_id): Path<String>,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let result = match parse_media_id(&media_id) {
        Ok(media_id) => state.access.view_url(identity.user_id, media_id).await,
        Err(error) => Err(error),
    };

    match result {
        Ok(media) => {
            tracing::info!(
                request_id = %request_id,
                event_code = "media_view_url_issued",
                "media request completed"
            );
            (StatusCode::OK, Json(MediaAccessResponse::from(media))).into_response()
        }
        Err(error) => MediaHttpError { error, request_id }.into_response(),
    }
}

async fn download(
    State(state): State<MediaHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(media_id): Path<String>,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let result = match parse_media_id(&media_id) {
        Ok(media_id) => state.access.download_url(identity.user_id, media_id).await,
        Err(error) => Err(error),
    };

    match result {
        Ok(get) => match HeaderValue::from_str(&get.url) {
            Ok(location) => {
                tracing::info!(
                    request_id = %request_id,
                    event_code = "media_download_url_issued",
                    "media request completed"
                );
                let mut response = StatusCode::TEMPORARY_REDIRECT.into_response();
                response.headers_mut().insert(LOCATION, location);
                response
            }
            Err(_) => MediaHttpError {
                error: MediaError::ObjectStorageDegraded,
                request_id,
            }
            .into_response(),
        },
        Err(error) => MediaHttpError { error, request_id }.into_response(),
    }
}

fn parse_media_id(media_id: &str) -> Result<Uuid, MediaError> {
    Uuid::try_parse(media_id).map_err(|_| MediaError::RequestValidation)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadIntentCreateRequest {
    scope: MediaScopeRequest,
    target_id: Uuid,
    content_type: String,
    byte_size: u64,
    filename: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum MediaScopeRequest {
    Chat,
    Topic,
}

impl From<MediaScopeRequest> for MediaScope {
    fn from(scope: MediaScopeRequest) -> Self {
        match scope {
            MediaScopeRequest::Chat => Self::Chat,
            MediaScopeRequest::Topic => Self::Topic,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadFinalizeRequest {
    width: Option<u32>,
    height: Option<u32>,
}

#[derive(Serialize)]
struct UploadIntentResponse {
    upload: UploadIntentRecordResponse,
    put: PresignedPutResponse,
}

impl From<UploadIntentWithPresignedPut> for UploadIntentResponse {
    fn from(intent: UploadIntentWithPresignedPut) -> Self {
        Self {
            upload: intent.upload.into(),
            put: intent.put.into(),
        }
    }
}

#[derive(Serialize)]
struct UploadIntentRecordResponse {
    id: Uuid,
    scope: &'static str,
    target_id: Uuid,
    object_key: String,
    kind: &'static str,
    content_type: String,
    byte_size: u64,
    filename: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    expires_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

impl From<UploadIntentRecord> for UploadIntentRecordResponse {
    fn from(upload: UploadIntentRecord) -> Self {
        Self {
            id: upload.id,
            scope: scope_name(upload.scope),
            target_id: upload.target_id,
            object_key: upload.object_key,
            kind: kind_name(upload.kind),
            content_type: upload.content_type,
            byte_size: upload.byte_size,
            filename: upload.filename,
            expires_at: upload.expires_at,
            created_at: upload.created_at,
        }
    }
}

#[derive(Serialize)]
struct PresignedPutResponse {
    url: String,
    expires_in: u64,
}

impl From<PresignedPut> for PresignedPutResponse {
    fn from(put: PresignedPut) -> Self {
        Self {
            url: put.url,
            expires_in: put.expires_in.as_secs(),
        }
    }
}

#[derive(Serialize)]
struct UploadFinalizeResponse {
    scope: &'static str,
    status: &'static str,
    bound: bool,
    upload: ConfirmedUploadResponse,
    topic_media: Option<TopicMediaBindingResponse>,
    topic_status: Option<&'static str>,
}

impl From<UploadFinalizeResult> for UploadFinalizeResponse {
    fn from(result: UploadFinalizeResult) -> Self {
        match result {
            UploadFinalizeResult::Chat { upload } => Self {
                scope: scope_name(upload.scope),
                status: "confirmed",
                bound: false,
                upload: upload.into(),
                topic_media: None,
                topic_status: None,
            },
            UploadFinalizeResult::Topic {
                upload,
                topic_media,
                topic_status,
            } => Self {
                scope: scope_name(upload.scope),
                status: "bound",
                bound: true,
                upload: upload.into(),
                topic_media: Some(topic_media.into()),
                topic_status: Some(topic_status.as_str()),
            },
        }
    }
}

#[derive(Serialize)]
struct ConfirmedUploadResponse {
    id: Uuid,
    scope: &'static str,
    target_id: Uuid,
    object_key: String,
    kind: &'static str,
    content_type: String,
    byte_size: u64,
    duration: Option<u64>,
    filename: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    confirmed_at: OffsetDateTime,
}

impl From<ConfirmedUploadRecord> for ConfirmedUploadResponse {
    fn from(upload: ConfirmedUploadRecord) -> Self {
        Self {
            id: upload.id,
            scope: scope_name(upload.scope),
            target_id: upload.target_id,
            object_key: upload.object_key,
            kind: kind_name(upload.kind),
            content_type: upload.content_type,
            byte_size: upload.byte_size,
            duration: upload.duration_seconds,
            filename: upload.filename,
            confirmed_at: upload.confirmed_at,
        }
    }
}

#[derive(Serialize)]
struct TopicMediaBindingResponse {
    id: Uuid,
    topic_id: Uuid,
    media_upload_id: Uuid,
    object_key: String,
    content_type: String,
    width: Option<u32>,
    height: Option<u32>,
    byte_size: u64,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

impl From<TopicMediaBindingRecord> for TopicMediaBindingResponse {
    fn from(media: TopicMediaBindingRecord) -> Self {
        Self {
            id: media.id,
            topic_id: media.topic_id,
            media_upload_id: media.media_upload_id,
            object_key: media.object_key,
            content_type: media.content_type,
            width: media.width,
            height: media.height,
            byte_size: media.byte_size,
            created_at: media.created_at,
        }
    }
}

fn scope_name(scope: MediaScope) -> &'static str {
    match scope {
        MediaScope::Chat => "chat",
        MediaScope::Topic => "topic",
    }
}

fn kind_name(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "image",
        MediaKind::Video => "video",
        MediaKind::Audio => "audio",
    }
}

#[derive(Serialize)]
struct MediaAccessResponse {
    id: Uuid,
    media_upload_id: Uuid,
    url: String,
    content_type: String,
    byte_size: u64,
    width: Option<u32>,
    height: Option<u32>,
    duration: Option<u64>,
    filename: Option<String>,
    expires_in: u64,
}

impl From<MediaAccessUrl> for MediaAccessResponse {
    fn from(media: MediaAccessUrl) -> Self {
        Self {
            id: media.id,
            media_upload_id: media.media_upload_id,
            url: media.url,
            content_type: media.content_type,
            byte_size: media.byte_size,
            width: media.width,
            height: media.height,
            duration: media.duration_seconds,
            filename: media.filename,
            expires_in: media.expires_in.as_secs(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MediaHttpError {
    error: MediaError,
    request_id: Uuid,
}

impl IntoResponse for MediaHttpError {
    fn into_response(self) -> Response {
        let (status, code, message) = error_profile(self.error);
        tracing::warn!(
            request_id = %self.request_id,
            error_code = code,
            "media request rejected"
        );
        let mut response = error_response(status, code, message, self.request_id);
        if let MediaError::RateLimited { retry_after } = self.error {
            let seconds = retry_after
                .as_secs()
                .saturating_add(u64::from(retry_after.subsec_nanos() > 0))
                .max(1);
            if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
                response.headers_mut().insert(RETRY_AFTER, value);
            }
        }
        response
    }
}

fn error_profile(error: MediaError) -> (StatusCode, &'static str, &'static str) {
    match error {
        MediaError::RequestValidation => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "request_validation_failed",
            "요청 형식이 올바르지 않습니다.",
        ),
        MediaError::RateLimited { .. } => (
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limit_exceeded",
            "요청이 너무 많습니다. 잠시 후 다시 시도해 주세요.",
        ),
        MediaError::RateLimitUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "rate_limit_unavailable",
            "요청 제한 서비스를 사용할 수 없습니다.",
        ),
        MediaError::TargetNotAccessible => (
            StatusCode::FORBIDDEN,
            "media_not_accessible",
            "이 미디어에 접근할 수 없습니다.",
        ),
        MediaError::FinalizeConflict => (
            StatusCode::CONFLICT,
            "media_finalize_conflict",
            "미디어 업로드 상태가 요청과 충돌합니다.",
        ),
        MediaError::FinalizeValidation => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "media_finalize_validation_failed",
            "업로드된 미디어를 확인할 수 없습니다.",
        ),
        MediaError::DatabaseUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
            "데이터베이스를 사용할 수 없습니다.",
        ),
        MediaError::ObjectStorageDegraded => (
            StatusCode::SERVICE_UNAVAILABLE,
            "object_storage_degraded",
            "미디어 저장소를 사용할 수 없습니다.",
        ),
        MediaError::InvalidConfiguration => (
            StatusCode::SERVICE_UNAVAILABLE,
            "media_unavailable",
            "미디어 서비스를 사용할 수 없습니다.",
        ),
    }
}
