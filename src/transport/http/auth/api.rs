use std::{collections::BTreeSet, net::SocketAddr, sync::Arc};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, FromRef, Path, Request, State},
    http::{
        HeaderValue, StatusCode,
        header::{CACHE_CONTROL, LOCATION, REFERRER_POLICY, RETRY_AFTER, X_CONTENT_TYPE_OPTIONS},
    },
    response::{IntoResponse, Response},
    routing::{get, post},
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
const MAX_CALLBACK_QUERY_BYTES: usize = 8 * 1024;
const MAX_CALLBACK_PARAMETERS: usize = 16;
const MAX_CALLBACK_CODE_BYTES: usize = 4096;
const MAX_CALLBACK_METADATA_BYTES: usize = 1024;
const KAKAO_APP_RETURN_URI: &str = "jamye://oauth/kakao";
const GOOGLE_APP_RETURN_URI: &str = "jamye://oauth/google";

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
        .route("/api/v1/auth/oauth/{provider}/callback", get(callback))
        .route("/api/v1/auth/refresh", post(refresh))
        .route("/api/v1/auth/logout", post(logout))
        .with_state(state)
}

async fn callback(Path(provider): Path<String>, request: Request) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let result = callback_location(&provider, parts.uri.query()).and_then(callback_redirect);
    match result {
        Ok(response) => response,
        Err(error) => callback_error_response(error, request_id),
    }
}

fn callback_location(provider: &str, query: Option<&str>) -> Result<String, AuthError> {
    let app_return_uri = app_return_uri(provider)?;
    let callback = parse_callback_query(query)?;
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("state", &callback.state);
    match callback.outcome {
        CallbackOutcome::Code(code) => query.append_pair("code", &code),
        CallbackOutcome::Error(error) => query.append_pair("error", error),
    };
    Ok(format!("{app_return_uri}?{}", query.finish()))
}

fn app_return_uri(provider: &str) -> Result<&'static str, AuthError> {
    match provider {
        "kakao" => Ok(KAKAO_APP_RETURN_URI),
        "google" => Ok(GOOGLE_APP_RETURN_URI),
        _ => Err(AuthError::OAuthProviderNotSupported),
    }
}

fn callback_redirect(location: String) -> Result<Response, AuthError> {
    let mut response = StatusCode::FOUND.into_response();
    let location = HeaderValue::from_str(&location).map_err(|_| AuthError::RequestValidation)?;
    response.headers_mut().insert(LOCATION, location);
    apply_callback_safety_headers(&mut response);
    Ok(response)
}

fn callback_error_response(error: AuthError, request_id: Uuid) -> Response {
    let (status, code, message) = error_profile(error);
    let mut response = error_response(status, code, message, request_id);
    apply_callback_safety_headers(&mut response);
    response
}

fn apply_callback_safety_headers(response: &mut Response) {
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response
        .headers_mut()
        .insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
}

struct CallbackQuery {
    state: String,
    outcome: CallbackOutcome,
}

enum CallbackOutcome {
    Code(String),
    Error(&'static str),
}

fn parse_callback_query(query: Option<&str>) -> Result<CallbackQuery, AuthError> {
    let query = query.ok_or(AuthError::RequestValidation)?;
    if query.is_empty() || query.len() > MAX_CALLBACK_QUERY_BYTES {
        return Err(AuthError::RequestValidation);
    }

    let mut fields = BTreeSet::new();
    let mut code = None;
    let mut state = None;
    let mut provider_error = None;
    for pair in query.split('&') {
        if pair.is_empty() || fields.len() == MAX_CALLBACK_PARAMETERS {
            return Err(AuthError::RequestValidation);
        }
        let (raw_key, raw_value) = pair.split_once('=').ok_or(AuthError::RequestValidation)?;
        let key = strict_query_decode(raw_key, 64)?;
        let value_limit = match key.as_str() {
            "code" => MAX_CALLBACK_CODE_BYTES,
            "state" | "error" | "error_description" | "error_uri" | "scope" | "authuser"
            | "prompt" | "iss" => MAX_CALLBACK_METADATA_BYTES,
            _ => return Err(AuthError::RequestValidation),
        };
        let value = strict_query_decode(raw_value, value_limit)?;
        if !fields.insert(key.clone()) {
            return Err(AuthError::RequestValidation);
        }
        match key.as_str() {
            "code" => code = Some(value),
            "state" => state = Some(value),
            "error" => provider_error = Some(value),
            _ => {
                if value.is_empty() || value.chars().any(char::is_control) {
                    return Err(AuthError::RequestValidation);
                }
            }
        }
    }

    let state = state.ok_or(AuthError::RequestValidation)?;
    if !valid_callback_state(&state) {
        return Err(AuthError::RequestValidation);
    }
    match (code, provider_error) {
        (Some(code), None) if valid_callback_code(&code) => Ok(CallbackQuery {
            state,
            outcome: CallbackOutcome::Code(code),
        }),
        (None, Some(error)) if valid_callback_error(&error) => Ok(CallbackQuery {
            state,
            outcome: CallbackOutcome::Error(if error == "access_denied" {
                "access_denied"
            } else {
                "oauth_failed"
            }),
        }),
        _ => Err(AuthError::RequestValidation),
    }
}

fn strict_query_decode(value: &str, maximum_bytes: usize) -> Result<String, AuthError> {
    let mut decoded = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = match bytes[index] {
            b'+' => b' ',
            b'%' => {
                let high = *bytes.get(index + 1).ok_or(AuthError::RequestValidation)?;
                let low = *bytes.get(index + 2).ok_or(AuthError::RequestValidation)?;
                index += 2;
                (hex_value(high).ok_or(AuthError::RequestValidation)? << 4)
                    | hex_value(low).ok_or(AuthError::RequestValidation)?
            }
            byte => byte,
        };
        decoded.push(byte);
        if decoded.len() > maximum_bytes {
            return Err(AuthError::RequestValidation);
        }
        index += 1;
    }
    String::from_utf8(decoded).map_err(|_| AuthError::RequestValidation)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn valid_callback_state(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn valid_callback_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CALLBACK_CODE_BYTES
        && !value.chars().any(char::is_control)
}

fn valid_callback_error(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CALLBACK_METADATA_BYTES
        && !value.chars().any(char::is_control)
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
