//! Authenticated Axum boundary for notification history and owner-scoped reads.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{FromRef, Path, RawQuery, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use time::OffsetDateTime;
use url::form_urlencoded;
use uuid::Uuid;

use crate::{
    application::{
        auth::AccessTokenVerifier,
        notifications::{
            MAX_NOTIFICATION_PAGE_LIMIT, NotificationPageInput, NotificationsError,
            NotificationsService,
        },
    },
    ports::push::{NotificationArgs, NotificationPage, NotificationRecord},
    transport::http::auth::{AuthVerifierState, AuthenticatedAccess, error_response, request_id},
};

#[derive(Clone)]
pub struct NotificationsHttpState {
    service: Arc<NotificationsService>,
    verifier: AuthVerifierState,
}

impl NotificationsHttpState {
    pub fn new(service: Arc<NotificationsService>, verifier: Arc<dyn AccessTokenVerifier>) -> Self {
        Self {
            service,
            verifier: AuthVerifierState::new(verifier),
        }
    }
}

impl FromRef<NotificationsHttpState> for AuthVerifierState {
    fn from_ref(state: &NotificationsHttpState) -> Self {
        state.verifier.clone()
    }
}

pub fn router(state: NotificationsHttpState) -> Router {
    Router::new()
        .route("/api/v1/notifications", get(list_notifications))
        .route(
            "/api/v1/notifications/{notification_id}/read",
            post(mark_notification_read),
        )
        .with_state(state)
}

async fn list_notifications(
    State(state): State<NotificationsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    RawQuery(raw_query): RawQuery,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let input = parse_notification_page(raw_query.as_deref());
    let result = match input {
        Ok(input) => {
            state
                .service
                .list_notifications(identity.user_id, input)
                .await
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(page) => (StatusCode::OK, Json(NotificationPageResponse::from(page))).into_response(),
        Err(error) => NotificationsHttpError { error, request_id }.into_response(),
    }
}

async fn mark_notification_read(
    State(state): State<NotificationsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(notification_id): Path<String>,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let result = match Uuid::try_parse(&notification_id) {
        Ok(notification_id) => {
            state
                .service
                .mark_read(identity.user_id, notification_id)
                .await
        }
        Err(_) => Err(NotificationsError::RequestValidation),
    };
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => NotificationsHttpError { error, request_id }.into_response(),
    }
}

fn parse_notification_page(
    raw_query: Option<&str>,
) -> Result<NotificationPageInput, NotificationsError> {
    let mut after = None;
    let mut limit = None;
    for (key, value) in form_urlencoded::parse(raw_query.unwrap_or_default().as_bytes()) {
        match key.as_ref() {
            "after" if after.is_none() => {
                let value = value.into_owned();
                Uuid::try_parse(&value).map_err(|_| NotificationsError::RequestValidation)?;
                after = Some(value);
            }
            "limit" if limit.is_none() => {
                let value = value
                    .parse::<u32>()
                    .map_err(|_| NotificationsError::RequestValidation)?;
                if !(1..=MAX_NOTIFICATION_PAGE_LIMIT).contains(&value) {
                    return Err(NotificationsError::RequestValidation);
                }
                limit = Some(value);
            }
            _ => return Err(NotificationsError::RequestValidation),
        }
    }
    Ok(NotificationPageInput { after, limit })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NotificationsHttpError {
    error: NotificationsError,
    request_id: Uuid,
}

impl IntoResponse for NotificationsHttpError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self.error {
            NotificationsError::RequestValidation => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "request_validation_failed",
                "요청 형식이 올바르지 않습니다.",
            ),
            NotificationsError::NotificationNotFound => (
                StatusCode::NOT_FOUND,
                "notification_not_found",
                "알림을 찾을 수 없습니다.",
            ),
            NotificationsError::DatabaseUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
                "데이터베이스를 사용할 수 없습니다.",
            ),
        };
        tracing::warn!(
            request_id = %self.request_id,
            error_code = code,
            "notification request rejected"
        );
        error_response(status, code, message, self.request_id)
    }
}

#[derive(Serialize)]
struct NotificationPageResponse {
    items: Vec<NotificationResponse>,
    next_cursor: Option<String>,
    unread_count: u64,
}

impl From<NotificationPage> for NotificationPageResponse {
    fn from(page: NotificationPage) -> Self {
        Self {
            items: page
                .items
                .into_iter()
                .map(NotificationResponse::from)
                .collect(),
            next_cursor: page.next_cursor,
            unread_count: page.unread_count,
        }
    }
}

#[derive(Serialize)]
struct NotificationResponse {
    id: Uuid,
    #[serde(rename = "type")]
    notification_type: &'static str,
    args: NotificationArgs,
    topic_id: Option<Uuid>,
    conversation_id: Option<Uuid>,
    source_cursor: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    read_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

impl From<NotificationRecord> for NotificationResponse {
    fn from(notification: NotificationRecord) -> Self {
        Self {
            id: notification.id,
            notification_type: notification.notification_type.as_str(),
            args: notification.args,
            topic_id: notification.topic_id,
            conversation_id: notification.conversation_id,
            source_cursor: notification.source_cursor.map(|cursor| cursor.to_string()),
            read_at: notification.read_at,
            created_at: notification.created_at,
        }
    }
}
