//! Authenticated Axum boundary for chatroom list, history, and read markers.

use std::sync::Arc;

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{FromRef, Path, RawQuery, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use url::form_urlencoded;
use uuid::Uuid;

use crate::{
    application::{
        auth::AccessTokenVerifier,
        chatrooms::{
            ChatroomPageInput, ChatroomsError, ChatroomsService, HistoryPageInput, ReadCursorInput,
        },
    },
    domain::messaging::{MessageAttachment, MessageKind},
    ports::chatrooms::{
        ChatroomPage, ChatroomRecord, MessageHistoryPage, MessageHistoryRecord, ReadMarker,
    },
    transport::http::auth::{AuthVerifierState, AuthenticatedAccess, error_response, request_id},
};

const MAX_READ_BODY_BYTES: usize = 8 * 1024;

#[derive(Clone)]
pub struct ChatroomsHttpState {
    service: Arc<ChatroomsService>,
    verifier: AuthVerifierState,
}

impl ChatroomsHttpState {
    pub fn new(service: Arc<ChatroomsService>, verifier: Arc<dyn AccessTokenVerifier>) -> Self {
        Self {
            service,
            verifier: AuthVerifierState::new(verifier),
        }
    }
}

impl FromRef<ChatroomsHttpState> for AuthVerifierState {
    fn from_ref(state: &ChatroomsHttpState) -> Self {
        state.verifier.clone()
    }
}

pub fn router(state: ChatroomsHttpState) -> Router {
    Router::new()
        .route("/api/v1/groups/{group_id}/chatrooms", get(list_chatrooms))
        .route(
            "/api/v1/chatrooms/{chatroom_id}/messages",
            get(message_history),
        )
        .route("/api/v1/chatrooms/{chatroom_id}/read", post(mark_read))
        .with_state(state)
}

async fn list_chatrooms(
    State(state): State<ChatroomsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(group_id): Path<String>,
    RawQuery(raw_query): RawQuery,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let input = parse_uuid(&group_id).and_then(|group_id| {
        parse_page(raw_query.as_deref(), "after").map(|page| (group_id, page))
    });
    let result = match input {
        Ok((group_id, page)) => {
            state
                .service
                .list_chatrooms(
                    identity.user_id,
                    group_id,
                    ChatroomPageInput {
                        after: page.cursor,
                        limit: page.limit,
                    },
                )
                .await
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(page) => (StatusCode::OK, Json(ChatroomPageResponse::from(page))).into_response(),
        Err(error) => ChatroomsHttpError { error, request_id }.into_response(),
    }
}

async fn message_history(
    State(state): State<ChatroomsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(chatroom_id): Path<String>,
    RawQuery(raw_query): RawQuery,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let input = parse_uuid(&chatroom_id).and_then(|chatroom_id| {
        parse_page(raw_query.as_deref(), "before").map(|page| (chatroom_id, page))
    });
    let result = match input {
        Ok((chatroom_id, page)) => {
            state
                .service
                .message_history(
                    identity.user_id,
                    chatroom_id,
                    HistoryPageInput {
                        before: page.cursor,
                        limit: page.limit,
                    },
                )
                .await
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(page) => (StatusCode::OK, Json(MessagePageResponse::from(page))).into_response(),
        Err(error) => ChatroomsHttpError { error, request_id }.into_response(),
    }
}

async fn mark_read(
    State(state): State<ChatroomsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(chatroom_id): Path<String>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    let input = match parse_uuid(&chatroom_id) {
        Ok(chatroom_id) => parse_json::<ReadCursorBody>(body).await.map(|payload| {
            (
                chatroom_id,
                ReadCursorInput {
                    cursor: payload.cursor,
                },
            )
        }),
        Err(error) => Err(error),
    };
    let result = match input {
        Ok((chatroom_id, input)) => {
            state
                .service
                .mark_read(identity.user_id, chatroom_id, input)
                .await
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(marker) => (StatusCode::OK, Json(ReadMarkerResponse::from(marker))).into_response(),
        Err(error) => ChatroomsHttpError { error, request_id }.into_response(),
    }
}

async fn parse_json<T>(body: Body) -> Result<T, ChatroomsError>
where
    T: for<'de> Deserialize<'de>,
{
    let bytes = to_bytes(body, MAX_READ_BODY_BYTES)
        .await
        .map_err(|_| ChatroomsError::RequestValidation)?;
    serde_json::from_slice(&bytes).map_err(|_| ChatroomsError::RequestValidation)
}

fn parse_uuid(value: &str) -> Result<Uuid, ChatroomsError> {
    Uuid::try_parse(value).map_err(|_| ChatroomsError::RequestValidation)
}

fn parse_page(raw_query: Option<&str>, cursor_name: &str) -> Result<RawPage, ChatroomsError> {
    let mut cursor = None;
    let mut limit = None;
    for (key, value) in form_urlencoded::parse(raw_query.unwrap_or_default().as_bytes()) {
        match key.as_ref() {
            key if key == cursor_name && cursor.is_none() => cursor = Some(value.into_owned()),
            "limit" if limit.is_none() => {
                limit = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| ChatroomsError::RequestValidation)?,
                );
            }
            _ => return Err(ChatroomsError::RequestValidation),
        }
    }
    Ok(RawPage { cursor, limit })
}

struct RawPage {
    cursor: Option<String>,
    limit: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ChatroomsHttpError {
    error: ChatroomsError,
    request_id: Uuid,
}

impl IntoResponse for ChatroomsHttpError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self.error {
            ChatroomsError::RequestValidation => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "request_validation_failed",
                "요청 형식이 올바르지 않습니다.",
            ),
            ChatroomsError::MembershipRequired => (
                StatusCode::FORBIDDEN,
                "membership_required",
                "이 채팅방에 접근할 수 없습니다.",
            ),
            ChatroomsError::DatabaseUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
                "데이터베이스를 사용할 수 없습니다.",
            ),
        };
        tracing::warn!(
            request_id = %self.request_id,
            error_code = code,
            "chatroom request rejected"
        );
        error_response(status, code, message, self.request_id)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadCursorBody {
    cursor: String,
}

#[derive(Serialize)]
struct ChatroomResponse {
    id: Uuid,
    group_id: Uuid,
    #[serde(rename = "type")]
    chatroom_type: &'static str,
    topic_id: Option<Uuid>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

impl From<ChatroomRecord> for ChatroomResponse {
    fn from(chatroom: ChatroomRecord) -> Self {
        Self {
            id: chatroom.id,
            group_id: chatroom.group_id,
            chatroom_type: chatroom.chatroom_type.as_str(),
            topic_id: chatroom.topic_id,
            created_at: chatroom.created_at,
        }
    }
}

#[derive(Serialize)]
struct ChatroomPageResponse {
    items: Vec<ChatroomResponse>,
    next_cursor: Option<String>,
}

impl From<ChatroomPage> for ChatroomPageResponse {
    fn from(page: ChatroomPage) -> Self {
        Self {
            items: page.items.into_iter().map(ChatroomResponse::from).collect(),
            next_cursor: page.next_cursor,
        }
    }
}

#[derive(Serialize)]
struct MessageResponse {
    id: Uuid,
    chatroom_id: Uuid,
    sender_id: Option<Uuid>,
    sender_nickname: Option<String>,
    sender_avatar_url: Option<String>,
    client_msg_id: Option<Uuid>,
    body: Option<String>,
    #[serde(rename = "type")]
    message_type: MessageKind,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    media: Vec<MessageAttachment>,
}

impl From<MessageHistoryRecord> for MessageResponse {
    fn from(item: MessageHistoryRecord) -> Self {
        Self {
            id: item.message.id,
            chatroom_id: item.message.chatroom_id,
            sender_id: item.message.sender_id,
            sender_nickname: item.sender_nickname,
            sender_avatar_url: item.sender_avatar_url,
            client_msg_id: item.message.client_msg_id,
            body: item.message.body,
            message_type: item.message.message_type,
            created_at: item.message.created_at,
            media: item.message.media,
        }
    }
}

#[derive(Serialize)]
struct MessagePageResponse {
    items: Vec<MessageResponse>,
    next_cursor: Option<String>,
}

impl From<MessageHistoryPage> for MessagePageResponse {
    fn from(page: MessageHistoryPage) -> Self {
        Self {
            items: page.items.into_iter().map(MessageResponse::from).collect(),
            next_cursor: page.next_cursor,
        }
    }
}

#[derive(Serialize)]
struct ReadMarkerResponse {
    chatroom_id: Uuid,
    last_read_cursor: String,
    #[serde(with = "time::serde::rfc3339")]
    updated_at: OffsetDateTime,
}

impl From<ReadMarker> for ReadMarkerResponse {
    fn from(marker: ReadMarker) -> Self {
        Self {
            chatroom_id: marker.chatroom_id,
            last_read_cursor: marker.last_read_cursor.to_string(),
            updated_at: marker.updated_at,
        }
    }
}
