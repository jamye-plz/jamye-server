//! Thin Axum boundary for the C4 command and S1 delta recovery routes.

use std::sync::Arc;

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{FromRef, Path, RawQuery, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use url::form_urlencoded;
use uuid::Uuid;

use crate::{
    application::messaging::{
        DEFAULT_DELTA_LIMIT, DeltaInput, MAX_DELTA_LIMIT, MessagingError, MessagingService,
        SendMessageInput, SendMessageOutcome,
    },
    transport::http::auth::{AuthVerifierState, AuthenticatedAccess, error_response, request_id},
};

const MAX_REQUEST_BYTES: usize = 64 * 1024;
const IDEMPOTENCY_KEY: &str = "idempotency-key";
const CONTRACT_VERSION: &str = "x-jamye-contract-version";

#[derive(Clone)]
pub struct MessagingHttpState {
    service: Arc<MessagingService>,
    auth: AuthVerifierState,
}

impl MessagingHttpState {
    pub fn new(service: Arc<MessagingService>, auth: AuthVerifierState) -> Self {
        Self { service, auth }
    }
}

impl FromRef<MessagingHttpState> for AuthVerifierState {
    fn from_ref(state: &MessagingHttpState) -> Self {
        state.auth.clone()
    }
}

pub fn router(state: MessagingHttpState) -> Router {
    Router::new()
        .route(
            "/api/v1/chatrooms/{chatroom_id}/messages",
            post(create_message),
        )
        .route(
            "/api/v1/conversations/{conversation_id}/events",
            get(events),
        )
        .with_state(state)
}

async fn create_message(
    State(state): State<MessagingHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(chatroom_id): Path<String>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    let result = parse_message_request(chatroom_id, &parts.headers, body).await;
    let result = match result {
        Ok(input) => state.service.send_message(&identity, input).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(outcome) => message_response(outcome, request_id),
        Err(error) => MessagingHttpError { error, request_id }.into_response(),
    }
}

async fn events(
    State(state): State<MessagingHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(conversation_id): Path<String>,
    RawQuery(raw_query): RawQuery,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let result = parse_delta_input(conversation_id, raw_query.as_deref(), &parts.headers)
        .map(|input| (input.contract_version.clone(), input));
    let result = match result {
        Ok((version, input)) => state
            .service
            .events(&identity, input)
            .await
            .map(|page| (version, page)),
        Err(error) => Err(error),
    };
    match result {
        Ok((version, page)) => event_page_response(version, page),
        Err(error) => MessagingHttpError { error, request_id }.into_response(),
    }
}

async fn parse_message_request(
    chatroom_id: String,
    headers: &HeaderMap,
    body: Body,
) -> Result<SendMessageInput, MessagingError> {
    let chatroom_id =
        Uuid::try_parse(&chatroom_id).map_err(|_| MessagingError::RequestValidation)?;
    let idempotency_key = optional_uuid_header(headers, IDEMPOTENCY_KEY)?;
    let bytes = to_bytes(body, MAX_REQUEST_BYTES)
        .await
        .map_err(|_| MessagingError::RequestValidation)?;
    let payload = serde_json::from_slice::<MessageCreate>(&bytes)
        .map_err(|_| MessagingError::RequestValidation)?;
    Ok(SendMessageInput {
        chatroom_id,
        client_msg_id: payload.client_msg_id,
        body: payload.body,
        media_upload_ids: payload
            .media
            .unwrap_or_default()
            .into_iter()
            .map(|media| media.media_upload_id)
            .collect(),
        idempotency_key,
    })
}

fn parse_delta_input(
    conversation_id: String,
    raw_query: Option<&str>,
    headers: &HeaderMap,
) -> Result<DeltaInput, MessagingError> {
    let conversation_id =
        Uuid::try_parse(&conversation_id).map_err(|_| MessagingError::RequestValidation)?;
    let (after, limit) = delta_query(raw_query)?;
    let contract_version = required_header(headers, CONTRACT_VERSION)?;
    Ok(DeltaInput {
        conversation_id,
        after,
        limit,
        contract_version,
    })
}

fn delta_query(raw_query: Option<&str>) -> Result<(Option<i64>, u32), MessagingError> {
    let mut after = None;
    let mut limit = None;
    for (key, value) in form_urlencoded::parse(raw_query.unwrap_or_default().as_bytes()) {
        match key.as_ref() {
            "after" if after.is_none() => {
                let cursor = value
                    .parse::<i64>()
                    .map_err(|_| MessagingError::RequestValidation)?;
                if cursor < 0 {
                    return Err(MessagingError::RequestValidation);
                }
                after = Some(cursor);
            }
            "limit" if limit.is_none() => {
                limit = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| MessagingError::RequestValidation)?,
                );
            }
            _ => return Err(MessagingError::RequestValidation),
        }
    }
    let limit = limit.unwrap_or(DEFAULT_DELTA_LIMIT);
    if !(1..=MAX_DELTA_LIMIT).contains(&limit) {
        return Err(MessagingError::RequestValidation);
    }
    Ok((after, limit))
}

fn optional_uuid_header(
    headers: &HeaderMap,
    name: &'static str,
) -> Result<Option<Uuid>, MessagingError> {
    let mut values = headers.get_all(name).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(MessagingError::IdempotencyKeyMismatch);
    }
    value
        .to_str()
        .ok()
        .and_then(|value| Uuid::try_parse(value).ok())
        .map(Some)
        .ok_or(MessagingError::IdempotencyKeyMismatch)
}

fn required_header(headers: &HeaderMap, name: &'static str) -> Result<String, MessagingError> {
    let mut values = headers.get_all(name).iter();
    let value = values
        .next()
        .ok_or(MessagingError::ContractUpgradeRequired)?;
    if values.next().is_some() {
        return Err(MessagingError::ContractUpgradeRequired);
    }
    value
        .to_str()
        .map(str::to_owned)
        .map_err(|_| MessagingError::ContractUpgradeRequired)
}

fn message_response(outcome: SendMessageOutcome, request_id: Uuid) -> Response {
    let (status, outcome_name, message) = match outcome {
        SendMessageOutcome::Created(message) => (StatusCode::CREATED, "created", message),
        SendMessageOutcome::Existing(message) => (StatusCode::OK, "existing", message),
    };
    tracing::info!(
        request_id = %request_id,
        outcome = outcome_name,
        message_id = %message.id,
        "message command completed"
    );
    (status, Json(message)).into_response()
}

fn event_page_response(version: String, page: crate::domain::messaging::EventPage) -> Response {
    let mut response = (StatusCode::OK, Json(page)).into_response();
    if let Ok(version) = HeaderValue::from_str(&version) {
        response.headers_mut().insert(CONTRACT_VERSION, version);
    }
    response
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MessagingHttpError {
    error: MessagingError,
    request_id: Uuid,
}

impl IntoResponse for MessagingHttpError {
    fn into_response(self) -> Response {
        let (status, code, message) = error_profile(self.error);
        tracing::warn!(
            request_id = %self.request_id,
            error_code = code,
            "messaging request rejected"
        );
        error_response(status, code, message, self.request_id)
    }
}

fn error_profile(error: MessagingError) -> (StatusCode, &'static str, &'static str) {
    match error {
        MessagingError::RequestValidation => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "request_validation_failed",
            "요청 형식이 올바르지 않습니다.",
        ),
        MessagingError::MessageContentRequired => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "message_content_required",
            "메시지 본문 또는 미디어가 필요합니다.",
        ),
        MessagingError::IdempotencyKeyMismatch => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "idempotency_key_mismatch",
            "Idempotency-Key가 client_msg_id와 일치하지 않습니다.",
        ),
        MessagingError::MediaNotAvailable => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "media_not_available",
            "미디어 메시지는 아직 사용할 수 없습니다.",
        ),
        MessagingError::MembershipRequired => (
            StatusCode::FORBIDDEN,
            "membership_required",
            "이 그룹에 접근할 수 없습니다.",
        ),
        MessagingError::IdempotencyConflict => (
            StatusCode::CONFLICT,
            "idempotency_conflict",
            "같은 메시지 키에 다른 내용이 사용되었습니다.",
        ),
        MessagingError::ContractUpgradeRequired => (
            StatusCode::UPGRADE_REQUIRED,
            "contract_upgrade_required",
            "지원되는 계약 버전으로 앱을 업데이트해 주세요.",
        ),
        MessagingError::DatabaseUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
            "데이터베이스를 사용할 수 없습니다.",
        ),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageCreate {
    client_msg_id: Uuid,
    body: Option<String>,
    media: Option<Vec<MediaRef>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MediaRef {
    media_upload_id: Uuid,
}
