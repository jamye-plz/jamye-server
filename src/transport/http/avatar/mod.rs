//! HTTP boundary for avatar upload (U4, U5) and public read (U6).

use std::{net::SocketAddr, sync::Arc};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, FromRef, Path, Request, State},
    http::{
        HeaderValue, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE, RETRY_AFTER, X_CONTENT_TYPE_OPTIONS},
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
        avatar::{AvatarError, AvatarService, AvatarUploadInput, AvatarUploadIntent},
    },
    ports::auth::UserProfile,
    transport::http::auth::{AuthVerifierState, AuthenticatedAccess, error_response, request_id},
};

const MAX_REQUEST_BYTES: usize = 8 * 1024;
const IMMUTABLE_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

#[derive(Clone)]
pub struct AvatarHttpState {
    service: Arc<AvatarService>,
    verifier: AuthVerifierState,
}

impl AvatarHttpState {
    pub fn new(service: Arc<AvatarService>, verifier: Arc<dyn AccessTokenVerifier>) -> Self {
        Self {
            service,
            verifier: AuthVerifierState::new(verifier),
        }
    }
}

impl FromRef<AvatarHttpState> for AuthVerifierState {
    fn from_ref(state: &AvatarHttpState) -> Self {
        state.verifier.clone()
    }
}

/// Mounted only when `JAMYE_AVATAR_PUBLIC_BASE_URL` is configured.
pub fn router(state: AvatarHttpState) -> Router {
    Router::new()
        .route("/api/v1/me/avatar/uploads", post(create_upload))
        .route(
            "/api/v1/me/avatar/uploads/{upload_id}/finalize",
            post(finalize_upload),
        )
        .route("/api/v1/avatars/{avatar_id}", get(read_avatar))
        .with_state(state)
}

async fn create_upload(
    State(state): State<AvatarHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    let result = match parse_json::<UploadCreateBody>(body).await {
        Ok(payload) => {
            state
                .service
                .create_upload(
                    identity.user_id,
                    AvatarUploadInput {
                        content_type: payload.content_type,
                        byte_size: payload.byte_size,
                    },
                )
                .await
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(intent) => {
            tracing::info!(
                request_id = %request_id,
                event_code = "avatar_upload_intent_created",
                "avatar request completed"
            );
            (
                StatusCode::CREATED,
                Json(UploadIntentResponse::from(intent)),
            )
                .into_response()
        }
        Err(error) => AvatarHttpError {
            error,
            request_id,
            no_store: false,
        }
        .into_response(),
    }
}

async fn finalize_upload(
    State(state): State<AvatarHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(upload_id): Path<String>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    let result = match parse_finalize(&upload_id, body).await {
        Ok(upload_id) => state.service.finalize(identity.user_id, upload_id).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(profile) => {
            tracing::info!(
                request_id = %request_id,
                event_code = "avatar_upload_finalized",
                "avatar request completed"
            );
            (StatusCode::OK, Json(UserResponse::from(profile))).into_response()
        }
        Err(error) => AvatarHttpError {
            error,
            request_id,
            no_store: false,
        }
        .into_response(),
    }
}

async fn read_avatar(
    State(state): State<AvatarHttpState>,
    Path(avatar_id): Path<String>,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    // Same peer-address key as the other unauthenticated limiters; behind a reverse
    // proxy this is the proxy address, which is a known MVP limitation.
    let rate_key = parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(address)| format!("ip:{}", address.ip()))
        .unwrap_or_else(|| "ip:unknown".to_owned());
    match state.service.read_public(&rate_key, &avatar_id).await {
        Ok(image) => (
            StatusCode::OK,
            [
                (CONTENT_TYPE, HeaderValue::from_static("image/jpeg")),
                (
                    CACHE_CONTROL,
                    HeaderValue::from_static(IMMUTABLE_CACHE_CONTROL),
                ),
                (X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")),
            ],
            image.bytes,
        )
            .into_response(),
        Err(error) => AvatarHttpError {
            error,
            request_id,
            no_store: true,
        }
        .into_response(),
    }
}

async fn parse_finalize(upload_id: &str, body: Body) -> Result<Uuid, AvatarError> {
    let upload_id = Uuid::try_parse(upload_id).map_err(|_| AvatarError::RequestValidation)?;
    parse_json::<EmptyBody>(body).await?;
    Ok(upload_id)
}

async fn parse_json<T>(body: Body) -> Result<T, AvatarError>
where
    T: DeserializeOwned,
{
    let bytes = to_bytes(body, MAX_REQUEST_BYTES)
        .await
        .map_err(|_| AvatarError::RequestValidation)?;
    serde_json::from_slice(&bytes).map_err(|_| AvatarError::RequestValidation)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadCreateBody {
    content_type: String,
    byte_size: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyBody {}

#[derive(Serialize)]
struct UploadIntentResponse {
    upload_id: Uuid,
    presigned_put: PresignedPutResponse,
}

#[derive(Serialize)]
struct PresignedPutResponse {
    url: String,
    expires_in: u64,
}

impl From<AvatarUploadIntent> for UploadIntentResponse {
    fn from(intent: AvatarUploadIntent) -> Self {
        Self {
            upload_id: intent.upload_id,
            presigned_put: PresignedPutResponse {
                url: intent.put.url,
                expires_in: intent.put.expires_in.as_secs(),
            },
        }
    }
}

#[derive(Serialize)]
struct UserResponse {
    id: Uuid,
    provider: String,
    nickname: String,
    avatar_url: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

impl From<UserProfile> for UserResponse {
    fn from(profile: UserProfile) -> Self {
        Self {
            id: profile.id,
            provider: profile.provider,
            nickname: profile.nickname,
            avatar_url: profile.avatar_url,
            created_at: profile.created_at,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AvatarHttpError {
    error: AvatarError,
    request_id: Uuid,
    /// Public-read errors must never be cached, so a replaced avatar id cannot linger.
    no_store: bool,
}

impl IntoResponse for AvatarHttpError {
    fn into_response(self) -> Response {
        let (status, code, message) = error_profile(self.error);
        tracing::warn!(
            request_id = %self.request_id,
            error_code = code,
            "avatar request rejected"
        );
        let mut response = error_response(status, code, message, self.request_id);
        if let AvatarError::RateLimited { retry_after } = self.error {
            let seconds = retry_after
                .as_secs()
                .saturating_add(u64::from(retry_after.subsec_nanos() > 0))
                .max(1);
            if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
                response.headers_mut().insert(RETRY_AFTER, value);
            }
        }
        if self.no_store {
            response
                .headers_mut()
                .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
        }
        response
    }
}

fn error_profile(error: AvatarError) -> (StatusCode, &'static str, &'static str) {
    match error {
        AvatarError::RequestValidation => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "request_validation_failed",
            "요청 형식이 올바르지 않습니다.",
        ),
        AvatarError::RateLimited { .. } => (
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limit_exceeded",
            "요청이 너무 많습니다. 잠시 후 다시 시도해 주세요.",
        ),
        AvatarError::RateLimitUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "rate_limit_unavailable",
            "요청 제한 서비스를 사용할 수 없습니다.",
        ),
        AvatarError::AccountNotActive => (
            StatusCode::UNAUTHORIZED,
            "authentication_required",
            "인증이 필요합니다.",
        ),
        AvatarError::UploadNotFound => (
            StatusCode::NOT_FOUND,
            "avatar_upload_not_found",
            "프로필 사진 업로드를 찾을 수 없습니다.",
        ),
        AvatarError::UploadNotPending => (
            StatusCode::CONFLICT,
            "avatar_upload_not_pending",
            "프로필 사진 업로드를 더 이상 확정할 수 없습니다.",
        ),
        AvatarError::ObjectInvalid => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "avatar_object_invalid",
            "업로드된 프로필 사진을 확인할 수 없습니다.",
        ),
        AvatarError::AvatarNotFound => (
            StatusCode::NOT_FOUND,
            "avatar_not_found",
            "프로필 사진을 찾을 수 없습니다.",
        ),
        AvatarError::DatabaseUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
            "데이터베이스를 사용할 수 없습니다.",
        ),
        AvatarError::ObjectStorageDegraded => (
            StatusCode::SERVICE_UNAVAILABLE,
            "object_storage_degraded",
            "미디어 저장소를 사용할 수 없습니다.",
        ),
        AvatarError::InvalidConfiguration => (
            StatusCode::SERVICE_UNAVAILABLE,
            "avatar_unavailable",
            "프로필 사진 서비스를 사용할 수 없습니다.",
        ),
    }
}
