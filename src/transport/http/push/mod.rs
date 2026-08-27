//! Authenticated Axum boundary for Expo installation lifecycle operations.

use std::sync::Arc;

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{FromRef, Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{post, put},
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    application::{
        auth::AccessTokenVerifier,
        push::{
            ExpoInstallationCreateInput, ExpoInstallationPutInput, PushError, PushInstallation,
            PushService,
        },
    },
    transport::http::auth::{AuthVerifierState, AuthenticatedAccess, error_response, request_id},
};

const MAX_PUSH_BODY_BYTES: usize = 16 * 1024;

#[derive(Clone)]
pub struct PushHttpState {
    service: Arc<PushService>,
    verifier: AuthVerifierState,
}

impl PushHttpState {
    pub fn new(service: Arc<PushService>, verifier: Arc<dyn AccessTokenVerifier>) -> Self {
        Self {
            service,
            verifier: AuthVerifierState::new(verifier),
        }
    }
}

impl FromRef<PushHttpState> for AuthVerifierState {
    fn from_ref(state: &PushHttpState) -> Self {
        state.verifier.clone()
    }
}

pub fn router(state: PushHttpState) -> Router {
    Router::new()
        .route("/api/v1/push/installations", post(upsert_installation))
        .route(
            "/api/v1/push/installations/{installation_id}",
            put(update_installation).delete(delete_installation),
        )
        .with_state(state)
}

async fn upsert_installation(
    State(state): State<PushHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    let result = match parse_json::<ExpoInstallationCreateBody>(body).await {
        Ok(payload) => {
            let input = ExpoInstallationCreateInput {
                platform: payload.platform,
                environment: payload.environment,
                installation_id: payload.installation_id,
                expo_token: payload.expo_token,
                message_preview_enabled: payload.message_preview_enabled,
            };
            state
                .service
                .upsert_installation(identity.user_id, input)
                .await
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(outcome) => {
            let status = if outcome.created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            (
                status,
                Json(PushInstallationResponse::from(outcome.installation)),
            )
                .into_response()
        }
        Err(error) => PushHttpError { error, request_id }.into_response(),
    }
}

async fn update_installation(
    State(state): State<PushHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(installation_id): Path<String>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    let result = match parse_json::<ExpoInstallationPutBody>(body).await {
        Ok(payload) => {
            let input = ExpoInstallationPutInput {
                expo_token: payload.expo_token,
                message_preview_enabled: payload.message_preview_enabled,
            };
            state
                .service
                .update_installation(identity.user_id, installation_id, input)
                .await
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(installation) => (
            StatusCode::OK,
            Json(PushInstallationResponse::from(installation)),
        )
            .into_response(),
        Err(error) => PushHttpError { error, request_id }.into_response(),
    }
}

async fn delete_installation(
    State(state): State<PushHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(installation_id): Path<String>,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    match state
        .service
        .delete_installation(identity.user_id, installation_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => PushHttpError { error, request_id }.into_response(),
    }
}

async fn parse_json<T>(body: Body) -> Result<T, PushError>
where
    T: for<'de> Deserialize<'de>,
{
    let bytes = to_bytes(body, MAX_PUSH_BODY_BYTES)
        .await
        .map_err(|_| PushError::RequestValidation)?;
    serde_json::from_slice(&bytes).map_err(|_| PushError::RequestValidation)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PushHttpError {
    error: PushError,
    request_id: Uuid,
}

impl IntoResponse for PushHttpError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self.error {
            PushError::RequestValidation => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "request_validation_failed",
                "요청 형식이 올바르지 않습니다.",
            ),
            PushError::InstallationNotFound => (
                StatusCode::NOT_FOUND,
                "push_installation_not_found",
                "푸시 설치를 찾을 수 없습니다.",
            ),
            PushError::DatabaseUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
                "데이터베이스를 사용할 수 없습니다.",
            ),
        };
        tracing::warn!(
            request_id = %self.request_id,
            error_code = code,
            "push installation request rejected"
        );
        error_response(status, code, message, self.request_id)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpoInstallationCreateBody {
    platform: String,
    environment: String,
    installation_id: String,
    expo_token: String,
    #[serde(default)]
    message_preview_enabled: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpoInstallationPutBody {
    expo_token: String,
    #[serde(default)]
    message_preview_enabled: Option<bool>,
}

#[derive(Serialize)]
struct PushInstallationResponse {
    installation_id: String,
    platform: &'static str,
    environment: &'static str,
    provider: &'static str,
    message_preview_enabled: bool,
    #[serde(with = "time::serde::rfc3339")]
    last_seen_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    disabled_at: Option<OffsetDateTime>,
}

impl From<PushInstallation> for PushInstallationResponse {
    fn from(installation: PushInstallation) -> Self {
        Self {
            installation_id: installation.installation_id,
            platform: installation.platform.as_str(),
            environment: installation.environment.as_str(),
            provider: installation.provider.as_str(),
            message_preview_enabled: installation.message_preview_enabled,
            last_seen_at: installation.last_seen_at,
            disabled_at: installation.disabled_at,
        }
    }
}
