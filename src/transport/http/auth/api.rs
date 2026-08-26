use std::{net::SocketAddr, sync::Arc};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, FromRef, Path, Request, State},
    http::{HeaderValue, StatusCode, header::RETRY_AFTER},
    response::{IntoResponse, Response},
    routing::post,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    application::auth::{
        AccessTokenVerifier, AuthError, AuthService, AuthorizeInput, ExchangeInput, TokenPair,
    },
    transport::http::auth::{AuthVerifierState, AuthenticatedAccess, error_response, request_id},
};

const MAX_AUTH_BODY_BYTES: usize = 16 * 1024;

#[derive(Clone)]
pub struct AuthHttpState {
    service: Arc<AuthService>,
    verifier: AuthVerifierState,
}

impl AuthHttpState {
    pub fn new(service: Arc<AuthService>, verifier: Arc<dyn AccessTokenVerifier>) -> Self {
        Self {
            service,
            verifier: AuthVerifierState::new(verifier),
        }
    }
}

impl FromRef<AuthHttpState> for AuthVerifierState {
    fn from_ref(state: &AuthHttpState) -> Self {
        state.verifier.clone()
    }
}

pub fn router(state: AuthHttpState) -> Router {
    Router::new()
        .route("/api/v1/auth/oauth/{provider}/authorize", post(authorize))
        .route("/api/v1/auth/oauth/{provider}/exchange", post(exchange))
        .route("/api/v1/auth/refresh", post(refresh))
        .route("/api/v1/auth/logout", post(logout))
        .with_state(state)
}

async fn authorize(
    State(state): State<AuthHttpState>,
    Path(provider): Path<String>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    let network_subject = network_subject(&parts);
    let result = parse_json::<OAuthAuthorizeBody>(body)
        .await
        .map(|body| AuthorizeInput {
            redirect_uri: body.redirect_uri,
            code_challenge: body.code_challenge,
            code_challenge_method: body.code_challenge_method,
        });
    let result = match result {
        Ok(input) => {
            state
                .service
                .authorize(&provider, input, &network_subject)
                .await
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(output) => (
            StatusCode::OK,
            Json(OAuthAuthorizeResponse {
                authorization_url: output.authorization_url,
                state: output.state,
                expires_in_seconds: output.expires_in_seconds,
            }),
        )
            .into_response(),
        Err(error) => AuthHttpError { error, request_id }.into_response(),
    }
}

async fn exchange(
    State(state): State<AuthHttpState>,
    Path(provider): Path<String>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    let network_subject = network_subject(&parts);
    let result = parse_json::<OAuthExchangeBody>(body)
        .await
        .map(|body| ExchangeInput {
            authorization_code: body.authorization_code,
            state: body.state,
            code_verifier: body.code_verifier,
            redirect_uri: body.redirect_uri,
        });
    let result = match result {
        Ok(input) => {
            state
                .service
                .exchange(&provider, input, &network_subject)
                .await
        }
        Err(error) => Err(error),
    };
    token_pair_result(result, request_id)
}

async fn refresh(State(state): State<AuthHttpState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    let result = parse_json::<RefreshBody>(body).await;
    let result = match result {
        Ok(input) => state.service.refresh(&input.refresh_token).await,
        Err(error) => Err(error),
    };
    token_pair_result(result, request_id)
}

async fn logout(
    State(state): State<AuthHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    match state
        .service
        .logout(identity.session_id, &format!("user:{}", identity.user_id))
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => AuthHttpError { error, request_id }.into_response(),
    }
}

async fn parse_json<T>(body: Body) -> Result<T, AuthError>
where
    T: for<'de> Deserialize<'de>,
{
    let bytes = to_bytes(body, MAX_AUTH_BODY_BYTES)
        .await
        .map_err(|_| AuthError::RequestValidation)?;
    serde_json::from_slice(&bytes).map_err(|_| AuthError::RequestValidation)
}

fn network_subject(parts: &axum::http::request::Parts) -> String {
    parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(address)| format!("ip:{}", address.ip()))
        .unwrap_or_else(|| "ip:unavailable".to_owned())
}

fn token_pair_result(result: Result<TokenPair, AuthError>, request_id: Uuid) -> Response {
    match result {
        Ok(pair) => (StatusCode::OK, Json(TokenPairResponse::from(pair))).into_response(),
        Err(error) => AuthHttpError { error, request_id }.into_response(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AuthHttpError {
    error: AuthError,
    request_id: Uuid,
}

impl IntoResponse for AuthHttpError {
    fn into_response(self) -> Response {
        let (status, code, message) = error_profile(self.error);
        tracing::warn!(
            request_id = %self.request_id,
            error_code = code,
            "authentication request rejected"
        );
        let mut response = error_response(status, code, message, self.request_id);
        if let AuthError::RateLimited { retry_after } = self.error {
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

fn error_profile(error: AuthError) -> (StatusCode, &'static str, &'static str) {
    match error {
        AuthError::RequestValidation => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "request_validation_failed",
            "요청 형식이 올바르지 않습니다.",
        ),
        AuthError::OAuthProviderNotSupported => (
            StatusCode::NOT_FOUND,
            "oauth_provider_not_supported",
            "지원하지 않는 로그인 제공자입니다.",
        ),
        AuthError::OAuthProviderNotAvailable => (
            StatusCode::NOT_FOUND,
            "oauth_provider_not_available",
            "현재 사용할 수 없는 로그인 제공자입니다.",
        ),
        AuthError::OAuthAuthorizeInvalid => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "oauth_authorize_invalid",
            "OAuth 인증 요청을 시작할 수 없습니다.",
        ),
        AuthError::OAuthExchangeInvalid => (
            StatusCode::UNAUTHORIZED,
            "oauth_exchange_invalid",
            "OAuth 인증 요청을 확인할 수 없습니다.",
        ),
        AuthError::OAuthCoordinationUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "oauth_coordination_unavailable",
            "OAuth 인증 조정 저장소를 사용할 수 없습니다.",
        ),
        AuthError::OAuthProviderUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "oauth_provider_unavailable",
            "로그인 제공자를 일시적으로 사용할 수 없습니다.",
        ),
        AuthError::RateLimited { .. } => (
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limit_exceeded",
            "요청이 너무 많습니다. 잠시 후 다시 시도해 주세요.",
        ),
        AuthError::RateLimitUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "rate_limit_unavailable",
            "요청 제한 서비스를 사용할 수 없습니다.",
        ),
        AuthError::RefreshTokenInvalid => (
            StatusCode::UNAUTHORIZED,
            "refresh_token_invalid",
            "다시 로그인해 주세요.",
        ),
        AuthError::RefreshTokenReused => (
            StatusCode::UNAUTHORIZED,
            "refresh_token_reused",
            "다시 로그인해 주세요.",
        ),
        AuthError::DatabaseUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
            "데이터베이스를 사용할 수 없습니다.",
        ),
        AuthError::TokenIssuanceUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "authentication_unavailable",
            "인증 토큰을 발급할 수 없습니다.",
        ),
        AuthError::InvalidConfiguration => (
            StatusCode::SERVICE_UNAVAILABLE,
            "authentication_unavailable",
            "인증 서비스를 사용할 수 없습니다.",
        ),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OAuthAuthorizeBody {
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
}

#[derive(Serialize)]
struct OAuthAuthorizeResponse {
    authorization_url: String,
    state: String,
    expires_in_seconds: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OAuthExchangeBody {
    authorization_code: String,
    state: String,
    code_verifier: String,
    redirect_uri: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RefreshBody {
    refresh_token: String,
}

#[derive(Serialize)]
struct TokenPairResponse {
    token_type: &'static str,
    access_token: String,
    #[serde(with = "time::serde::rfc3339")]
    access_token_expires_at: OffsetDateTime,
    refresh_token: String,
    #[serde(with = "time::serde::rfc3339")]
    refresh_token_expires_at: OffsetDateTime,
}

impl From<TokenPair> for TokenPairResponse {
    fn from(pair: TokenPair) -> Self {
        Self {
            token_type: pair.token_type,
            access_token: pair.access_token,
            access_token_expires_at: pair.access_token_expires_at,
            refresh_token: pair.refresh_token,
            refresh_token_expires_at: pair.refresh_token_expires_at,
        }
    }
}
