//! HTTP boundary for reports (R1) and user blocks (B1-B3).

use std::sync::Arc;

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{FromRef, Path, Request, State},
    http::{HeaderValue, StatusCode, header::RETRY_AFTER},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    application::{
        auth::AccessTokenVerifier,
        moderation::{ModerationError, ModerationService, ReportInput},
    },
    transport::http::auth::{AuthVerifierState, AuthenticatedAccess, error_response, request_id},
};

const MAX_REQUEST_BYTES: usize = 8 * 1024;

#[derive(Clone)]
pub struct ModerationHttpState {
    service: Arc<ModerationService>,
    verifier: AuthVerifierState,
}

impl ModerationHttpState {
    pub fn new(service: Arc<ModerationService>, verifier: Arc<dyn AccessTokenVerifier>) -> Self {
        Self {
            service,
            verifier: AuthVerifierState::new(verifier),
        }
    }
}

impl FromRef<ModerationHttpState> for AuthVerifierState {
    fn from_ref(state: &ModerationHttpState) -> Self {
        state.verifier.clone()
    }
}

pub fn router(state: ModerationHttpState) -> Router {
    Router::new()
        .route("/api/v1/reports", post(create_report))
        .route("/api/v1/me/blocks", get(list_blocks))
        .route(
            "/api/v1/me/blocks/{user_id}",
            put(block_user).delete(unblock_user),
        )
        .with_state(state)
}

async fn create_report(
    State(state): State<ModerationHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    let result = match parse_json::<ReportBody>(body).await {
        Ok(payload) => {
            state
                .service
                .report(
                    identity.user_id,
                    ReportInput {
                        target_type: payload.target_type,
                        message_id: payload.message_id,
                        user_id: payload.user_id,
                        reason: payload.reason,
                    },
                )
                .await
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(record) => {
            tracing::info!(
                request_id = %request_id,
                event_code = "report_created",
                "moderation request completed"
            );
            (
                StatusCode::CREATED,
                Json(ReportResponse {
                    id: record.id,
                    status: "open",
                    created_at: record.created_at,
                }),
            )
                .into_response()
        }
        Err(error) => ModerationHttpError { error, request_id }.into_response(),
    }
}

async fn block_user(
    State(state): State<ModerationHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(user_id): Path<String>,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let result = match Uuid::try_parse(&user_id) {
        Ok(blocked_id) => state.service.block(identity.user_id, blocked_id).await,
        Err(_) => Err(ModerationError::RequestValidation),
    };
    match result {
        Ok(record) => (
            StatusCode::OK,
            Json(BlockResponse {
                user_id: record.blocked_id,
                blocked_at: record.blocked_at,
            }),
        )
            .into_response(),
        Err(error) => ModerationHttpError { error, request_id }.into_response(),
    }
}

async fn unblock_user(
    State(state): State<ModerationHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(user_id): Path<String>,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let result = match Uuid::try_parse(&user_id) {
        Ok(blocked_id) => state.service.unblock(identity.user_id, blocked_id).await,
        Err(_) => Err(ModerationError::RequestValidation),
    };
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => ModerationHttpError { error, request_id }.into_response(),
    }
}

async fn list_blocks(
    State(state): State<ModerationHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    match state.service.list_blocks(identity.user_id).await {
        Ok(blocked) => (
            StatusCode::OK,
            Json(BlockListResponse {
                items: blocked
                    .into_iter()
                    .map(|user| BlockedUserResponse {
                        user_id: user.user_id,
                        nickname: user.nickname,
                        avatar_url: user.avatar_url,
                        blocked_at: user.blocked_at,
                    })
                    .collect(),
            }),
        )
            .into_response(),
        Err(error) => ModerationHttpError { error, request_id }.into_response(),
    }
}

async fn parse_json<T>(body: Body) -> Result<T, ModerationError>
where
    T: DeserializeOwned,
{
    let bytes = to_bytes(body, MAX_REQUEST_BYTES)
        .await
        .map_err(|_| ModerationError::RequestValidation)?;
    serde_json::from_slice(&bytes).map_err(|_| ModerationError::RequestValidation)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportBody {
    target_type: String,
    message_id: Option<Uuid>,
    user_id: Option<Uuid>,
    reason: String,
}

#[derive(Serialize)]
struct ReportResponse {
    id: Uuid,
    status: &'static str,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

#[derive(Serialize)]
struct BlockResponse {
    user_id: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    blocked_at: OffsetDateTime,
}

#[derive(Serialize)]
struct BlockListResponse {
    items: Vec<BlockedUserResponse>,
}

#[derive(Serialize)]
struct BlockedUserResponse {
    user_id: Uuid,
    nickname: String,
    avatar_url: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    blocked_at: OffsetDateTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ModerationHttpError {
    error: ModerationError,
    request_id: Uuid,
}

impl IntoResponse for ModerationHttpError {
    fn into_response(self) -> Response {
        let (status, code, message) = error_profile(self.error);
        tracing::warn!(
            request_id = %self.request_id,
            error_code = code,
            "moderation request rejected"
        );
        let mut response = error_response(status, code, message, self.request_id);
        if let ModerationError::RateLimited { retry_after } = self.error {
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

fn error_profile(error: ModerationError) -> (StatusCode, &'static str, &'static str) {
    match error {
        ModerationError::RequestValidation => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "request_validation_failed",
            "요청 형식이 올바르지 않습니다.",
        ),
        ModerationError::ReportTargetNotFound => (
            StatusCode::NOT_FOUND,
            "report_target_not_found",
            "신고 대상을 찾을 수 없습니다.",
        ),
        ModerationError::ReportSelfTarget => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "report_self_target",
            "자신을 신고할 수 없습니다.",
        ),
        ModerationError::BlockSelfNotAllowed => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "block_self_not_allowed",
            "자신을 차단할 수 없습니다.",
        ),
        ModerationError::UserNotFound => (
            StatusCode::NOT_FOUND,
            "user_not_found",
            "사용자를 찾을 수 없습니다.",
        ),
        ModerationError::RateLimited { .. } => (
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limit_exceeded",
            "요청이 너무 많습니다. 잠시 후 다시 시도해 주세요.",
        ),
        ModerationError::RateLimitUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "rate_limit_unavailable",
            "요청 제한 서비스를 사용할 수 없습니다.",
        ),
        ModerationError::DatabaseUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
            "데이터베이스를 사용할 수 없습니다.",
        ),
    }
}
