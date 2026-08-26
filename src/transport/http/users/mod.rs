//! Authenticated current-user profile transport.

use std::sync::Arc;

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{FromRef, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Deserializer, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    application::{
        auth::AccessTokenVerifier,
        users::{PatchValue, UserError, UserPatch, UserService},
    },
    ports::auth::UserProfile,
    transport::http::auth::{AuthVerifierState, AuthenticatedAccess, error_response, request_id},
};

const MAX_PROFILE_BODY_BYTES: usize = 8 * 1024;

#[derive(Clone)]
pub struct UserHttpState {
    service: Arc<UserService>,
    verifier: AuthVerifierState,
}

impl UserHttpState {
    pub fn new(service: Arc<UserService>, verifier: Arc<dyn AccessTokenVerifier>) -> Self {
        Self {
            service,
            verifier: AuthVerifierState::new(verifier),
        }
    }
}

impl FromRef<UserHttpState> for AuthVerifierState {
    fn from_ref(state: &UserHttpState) -> Self {
        state.verifier.clone()
    }
}

pub fn router(state: UserHttpState) -> Router {
    Router::new()
        .route("/api/v1/me", get(get_me).patch(patch_me))
        .with_state(state)
}

async fn get_me(
    State(state): State<UserHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    profile_result(
        state.service.get(identity.user_id).await,
        request_id(&parts),
    )
}

async fn patch_me(
    State(state): State<UserHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    let result = parse_patch(body).await;
    let result = match result {
        Ok(patch) => state.service.update(identity.user_id, patch).await,
        Err(error) => Err(error),
    };
    profile_result(result, request_id)
}

async fn parse_patch(body: Body) -> Result<UserPatch, UserError> {
    let bytes = to_bytes(body, MAX_PROFILE_BODY_BYTES)
        .await
        .map_err(|_| UserError::RequestValidation)?;
    let body = serde_json::from_slice::<UserPatchBody>(&bytes)
        .map_err(|_| UserError::RequestValidation)?;
    Ok(UserPatch {
        nickname: body.nickname.into_patch(),
        avatar_url: body.avatar_url.into_patch(),
    })
}

fn profile_result(result: Result<UserProfile, UserError>, request_id: Uuid) -> Response {
    match result {
        Ok(profile) => (StatusCode::OK, Json(UserResponse::from(profile))).into_response(),
        Err(error) => UserHttpError { error, request_id }.into_response(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct UserHttpError {
    error: UserError,
    request_id: Uuid,
}

impl IntoResponse for UserHttpError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self.error {
            UserError::RequestValidation => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "request_validation_failed",
                "요청 형식이 올바르지 않습니다.",
            ),
            UserError::ProfileNotFound => (
                StatusCode::UNAUTHORIZED,
                "authentication_required",
                "인증이 필요합니다.",
            ),
            UserError::DatabaseUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
                "데이터베이스를 사용할 수 없습니다.",
            ),
        };
        tracing::warn!(
            request_id = %self.request_id,
            error_code = code,
            "profile request rejected"
        );
        error_response(status, code, message, self.request_id)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UserPatchBody {
    #[serde(default)]
    nickname: NullableField,
    #[serde(default)]
    avatar_url: NullableField,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum NullableField {
    Omitted,
    Null,
    Value(String),
}

impl Default for NullableField {
    fn default() -> Self {
        Self::Omitted
    }
}

impl<'de> Deserialize<'de> for NullableField {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<String>::deserialize(deserializer)
            .map(|value| value.map_or(Self::Null, Self::Value))
    }
}

impl NullableField {
    fn into_patch(self) -> PatchValue<String> {
        match self {
            Self::Omitted => PatchValue::Omitted,
            Self::Null => PatchValue::Null,
            Self::Value(value) => PatchValue::Value(value),
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
