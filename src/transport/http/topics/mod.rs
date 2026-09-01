//! Authenticated Axum boundary for topic lifecycle, timeline, unread, tags, and media.

use std::sync::Arc;

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{FromRef, Path, RawQuery, Request, State},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use url::form_urlencoded;
use uuid::Uuid;

use crate::{
    application::{
        auth::AccessTokenVerifier,
        topics::{
            TopicCreateInput, TopicDatePageInput, TopicMediaPageInput, TopicPageInput,
            TopicPatchInput, TopicTagInput, TopicTagPageInput, TopicTagsInput, TopicsError,
            TopicsService,
        },
        transactions::TransactionCompositions,
    },
    ports::topics::{
        TopicDatePage, TopicMediaPage, TopicMediaRecord, TopicPage, TopicRecord, TopicTagPage,
        TopicTagRecord,
    },
    transport::http::auth::{AuthVerifierState, AuthenticatedAccess, error_response, request_id},
};

const MAX_TOPIC_BODY_BYTES: usize = 32 * 1024;

#[derive(Clone)]
pub struct TopicsHttpState {
    service: Arc<TopicsService>,
    verifier: AuthVerifierState,
    compositions: Option<Arc<TransactionCompositions>>,
}

impl TopicsHttpState {
    pub fn new(service: Arc<TopicsService>, verifier: Arc<dyn AccessTokenVerifier>) -> Self {
        Self {
            service,
            verifier: AuthVerifierState::new(verifier),
            compositions: None,
        }
    }

    pub fn with_compositions(mut self, compositions: Arc<TransactionCompositions>) -> Self {
        self.compositions = Some(compositions);
        self
    }
}

impl FromRef<TopicsHttpState> for AuthVerifierState {
    fn from_ref(state: &TopicsHttpState) -> Self {
        state.verifier.clone()
    }
}

pub fn router(state: TopicsHttpState) -> Router {
    Router::new()
        .route(
            "/api/v1/groups/{group_id}/topics",
            get(list_topics).post(create_topic),
        )
        .route(
            "/api/v1/groups/{group_id}/topics/dates",
            get(list_topic_dates),
        )
        .route(
            "/api/v1/groups/{group_id}/topics/{topic_id}",
            get(get_topic).patch(patch_topic),
        )
        .route(
            "/api/v1/groups/{group_id}/topics/{topic_id}/tags",
            get(list_tags).put(replace_tags),
        )
        .route("/api/v1/topics/{topic_id}/media", get(list_media))
        .with_state(state)
}

async fn create_topic(
    State(state): State<TopicsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(group_id): Path<String>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    let input = parse_uuid(&group_id).and_then(|group_id| {
        parse_idempotency_key(&parts).map(|idempotency_key| (group_id, idempotency_key))
    });
    let input = match input {
        Ok((group_id, idempotency_key)) => {
            parse_json::<TopicCreateBody>(body).await.map(|payload| {
                (
                    group_id,
                    TopicCreateInput {
                        idempotency_key,
                        title: payload.title,
                    },
                )
            })
        }
        Err(error) => Err(error),
    };
    let result = match input {
        Ok((group_id, input)) => match &state.compositions {
            Some(compositions) => {
                compositions
                    .create_topic_http(identity.user_id, group_id, input)
                    .await
            }
            None => {
                state
                    .service
                    .create_topic(identity.user_id, group_id, input)
                    .await
            }
        },
        Err(error) => Err(error),
    };
    match result {
        Ok(outcome) => {
            let status = if outcome.was_created() {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            (status, Json(TopicResponse::from(outcome.into_topic()))).into_response()
        }
        Err(error) => TopicsHttpError { error, request_id }.into_response(),
    }
}

async fn list_topics(
    State(state): State<TopicsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(group_id): Path<String>,
    RawQuery(raw_query): RawQuery,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let input = parse_uuid(&group_id)
        .and_then(|group_id| parse_topic_page(raw_query.as_deref()).map(|input| (group_id, input)));
    let result = match input {
        Ok((group_id, input)) => {
            state
                .service
                .list_topics(identity.user_id, group_id, input)
                .await
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(page) => (StatusCode::OK, Json(TopicPageResponse::from(page))).into_response(),
        Err(error) => TopicsHttpError { error, request_id }.into_response(),
    }
}

async fn list_topic_dates(
    State(state): State<TopicsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(group_id): Path<String>,
    RawQuery(raw_query): RawQuery,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let input = parse_uuid(&group_id).and_then(|group_id| {
        parse_cursor_page(raw_query.as_deref()).map(|page| {
            (
                group_id,
                TopicDatePageInput {
                    after: page.after,
                    limit: page.limit,
                },
            )
        })
    });
    let result = match input {
        Ok((group_id, input)) => {
            state
                .service
                .list_topic_dates(identity.user_id, group_id, input)
                .await
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(page) => (StatusCode::OK, Json(TopicDatePageResponse::from(page))).into_response(),
        Err(error) => TopicsHttpError { error, request_id }.into_response(),
    }
}

async fn get_topic(
    State(state): State<TopicsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path((group_id, topic_id)): Path<(String, String)>,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let ids = parse_uuid(&group_id)
        .and_then(|group_id| parse_uuid(&topic_id).map(|topic_id| (group_id, topic_id)));
    let result = match ids {
        Ok((group_id, topic_id)) => {
            state
                .service
                .get_topic(identity.user_id, group_id, topic_id)
                .await
        }
        Err(error) => Err(error),
    };
    topic_result(result, request_id)
}

async fn patch_topic(
    State(state): State<TopicsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path((group_id, topic_id)): Path<(String, String)>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    let ids = parse_uuid(&group_id)
        .and_then(|group_id| parse_uuid(&topic_id).map(|topic_id| (group_id, topic_id)));
    let input = match ids {
        Ok((group_id, topic_id)) => parse_json::<TopicPatchBody>(body).await.map(|payload| {
            (
                group_id,
                topic_id,
                TopicPatchInput {
                    title: payload.title,
                    body: payload.body,
                },
            )
        }),
        Err(error) => Err(error),
    };
    let result = match input {
        Ok((group_id, topic_id, input)) => {
            state
                .service
                .patch_topic(identity.user_id, group_id, topic_id, input)
                .await
        }
        Err(error) => Err(error),
    };
    topic_result(result, request_id)
}

async fn replace_tags(
    State(state): State<TopicsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path((group_id, topic_id)): Path<(String, String)>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    let ids = parse_uuid(&group_id)
        .and_then(|group_id| parse_uuid(&topic_id).map(|topic_id| (group_id, topic_id)));
    let input = match ids {
        Ok((group_id, topic_id)) => parse_json::<TopicTagsBody>(body).await.map(|payload| {
            (
                group_id,
                topic_id,
                TopicTagsInput {
                    tags: payload
                        .tags
                        .into_iter()
                        .map(|tag| TopicTagInput {
                            tag: tag.tag,
                            source: tag.source,
                            confidence: tag.confidence,
                        })
                        .collect(),
                },
            )
        }),
        Err(error) => Err(error),
    };
    let result = match input {
        Ok((group_id, topic_id, input)) => {
            state
                .service
                .replace_tags(identity.user_id, group_id, topic_id, input)
                .await
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(page) => (StatusCode::OK, Json(TopicTagPageResponse::from(page))).into_response(),
        Err(error) => TopicsHttpError { error, request_id }.into_response(),
    }
}

async fn list_tags(
    State(state): State<TopicsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path((group_id, topic_id)): Path<(String, String)>,
    RawQuery(raw_query): RawQuery,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let ids = parse_uuid(&group_id)
        .and_then(|group_id| parse_uuid(&topic_id).map(|topic_id| (group_id, topic_id)));
    let input = ids.and_then(|(group_id, topic_id)| {
        parse_cursor_page(raw_query.as_deref()).map(|page| {
            (
                group_id,
                topic_id,
                TopicTagPageInput {
                    after: page.after,
                    limit: page.limit,
                },
            )
        })
    });
    let result = match input {
        Ok((group_id, topic_id, input)) => {
            state
                .service
                .list_tags(identity.user_id, group_id, topic_id, input)
                .await
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(page) => (StatusCode::OK, Json(TopicTagPageResponse::from(page))).into_response(),
        Err(error) => TopicsHttpError { error, request_id }.into_response(),
    }
}

async fn list_media(
    State(state): State<TopicsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(topic_id): Path<String>,
    RawQuery(raw_query): RawQuery,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let input = parse_uuid(&topic_id).and_then(|topic_id| {
        parse_cursor_page(raw_query.as_deref()).map(|page| {
            (
                topic_id,
                TopicMediaPageInput {
                    after: page.after,
                    limit: page.limit,
                },
            )
        })
    });
    let result = match input {
        Ok((topic_id, input)) => {
            state
                .service
                .list_media(identity.user_id, topic_id, input)
                .await
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(page) => (StatusCode::OK, Json(TopicMediaPageResponse::from(page))).into_response(),
        Err(error) => TopicsHttpError { error, request_id }.into_response(),
    }
}

async fn parse_json<T>(body: Body) -> Result<T, TopicsError>
where
    T: for<'de> Deserialize<'de>,
{
    let bytes = to_bytes(body, MAX_TOPIC_BODY_BYTES)
        .await
        .map_err(|_| TopicsError::RequestValidation)?;
    serde_json::from_slice(&bytes).map_err(|_| TopicsError::RequestValidation)
}

fn parse_idempotency_key(parts: &Parts) -> Result<Uuid, TopicsError> {
    let mut values = parts.headers.get_all("idempotency-key").iter();
    let value = values.next().ok_or(TopicsError::RequestValidation)?;
    if values.next().is_some() {
        return Err(TopicsError::RequestValidation);
    }
    value
        .to_str()
        .ok()
        .and_then(|value| Uuid::try_parse(value).ok())
        .ok_or(TopicsError::RequestValidation)
}

fn parse_uuid(value: &str) -> Result<Uuid, TopicsError> {
    Uuid::try_parse(value).map_err(|_| TopicsError::RequestValidation)
}

fn parse_topic_page(raw_query: Option<&str>) -> Result<TopicPageInput, TopicsError> {
    let mut after = None;
    let mut limit = None;
    let mut date = None;
    for (key, value) in form_urlencoded::parse(raw_query.unwrap_or_default().as_bytes()) {
        match key.as_ref() {
            "after" if after.is_none() => after = Some(value.into_owned()),
            "limit" if limit.is_none() => limit = Some(parse_limit(&value)?),
            "date" if date.is_none() => date = Some(value.into_owned()),
            _ => return Err(TopicsError::RequestValidation),
        }
    }
    Ok(TopicPageInput { after, limit, date })
}

fn parse_cursor_page(raw_query: Option<&str>) -> Result<RawCursorPage, TopicsError> {
    let mut after = None;
    let mut limit = None;
    for (key, value) in form_urlencoded::parse(raw_query.unwrap_or_default().as_bytes()) {
        match key.as_ref() {
            "after" if after.is_none() => after = Some(value.into_owned()),
            "limit" if limit.is_none() => limit = Some(parse_limit(&value)?),
            _ => return Err(TopicsError::RequestValidation),
        }
    }
    Ok(RawCursorPage { after, limit })
}

fn parse_limit(value: &str) -> Result<u32, TopicsError> {
    value
        .parse::<u32>()
        .map_err(|_| TopicsError::RequestValidation)
}

fn topic_result(result: Result<TopicRecord, TopicsError>, request_id: Uuid) -> Response {
    match result {
        Ok(topic) => (StatusCode::OK, Json(TopicResponse::from(topic))).into_response(),
        Err(error) => TopicsHttpError { error, request_id }.into_response(),
    }
}

struct RawCursorPage {
    after: Option<String>,
    limit: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TopicsHttpError {
    error: TopicsError,
    request_id: Uuid,
}

impl IntoResponse for TopicsHttpError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self.error {
            TopicsError::RequestValidation => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "request_validation_failed",
                "요청 형식이 올바르지 않습니다.",
            ),
            TopicsError::GroupNotFound => (
                StatusCode::NOT_FOUND,
                "group_not_found",
                "그룹을 찾을 수 없습니다.",
            ),
            TopicsError::MembershipRequired => (
                StatusCode::FORBIDDEN,
                "membership_required",
                "이 그룹에 접근할 수 없습니다.",
            ),
            TopicsError::TopicNotFound => (
                StatusCode::NOT_FOUND,
                "topic_not_found",
                "주제를 찾을 수 없습니다.",
            ),
            TopicsError::AuthorRequired => (
                StatusCode::FORBIDDEN,
                "topic_author_required",
                "주제 작성자만 수정할 수 있습니다.",
            ),
            TopicsError::TopicManageRequired => (
                StatusCode::FORBIDDEN,
                "topic_manage_required",
                "주제 태그를 수정할 권한이 없습니다.",
            ),
            TopicsError::IdempotencyConflict => (
                StatusCode::CONFLICT,
                "topic_idempotency_conflict",
                "같은 Idempotency-Key가 다른 요청에 사용되었습니다.",
            ),
            TopicsError::DatabaseUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
                "데이터베이스를 사용할 수 없습니다.",
            ),
        };
        tracing::warn!(
            request_id = %self.request_id,
            error_code = code,
            "topic request rejected"
        );
        error_response(status, code, message, self.request_id)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TopicCreateBody {
    title: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TopicPatchBody {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    body: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TopicTagsBody {
    tags: Vec<TopicTagBody>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TopicTagBody {
    tag: String,
    source: String,
    #[serde(default)]
    confidence: Option<f64>,
}

#[derive(Serialize)]
struct TopicResponse {
    id: Uuid,
    group_id: Uuid,
    author_id: Uuid,
    author_nickname: String,
    author_avatar_url: Option<String>,
    title: String,
    body: Option<String>,
    status: &'static str,
    tags: Vec<TopicTagResponse>,
    media: Vec<TopicMediaResponse>,
    chatroom_id: Uuid,
    unread: bool,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    updated_at: OffsetDateTime,
}

impl From<TopicRecord> for TopicResponse {
    fn from(topic: TopicRecord) -> Self {
        Self {
            id: topic.id,
            group_id: topic.group_id,
            author_id: topic.author_id,
            author_nickname: topic.author_nickname,
            author_avatar_url: topic.author_avatar_url,
            title: topic.title,
            body: topic.body,
            status: topic.status.as_str(),
            tags: topic.tags.into_iter().map(TopicTagResponse::from).collect(),
            media: topic
                .media
                .into_iter()
                .map(TopicMediaResponse::from)
                .collect(),
            chatroom_id: topic.chatroom_id,
            unread: topic.unread,
            created_at: topic.created_at,
            updated_at: topic.updated_at,
        }
    }
}

#[derive(Serialize)]
struct TopicPageResponse {
    items: Vec<TopicResponse>,
    next_cursor: Option<String>,
}

impl From<TopicPage> for TopicPageResponse {
    fn from(page: TopicPage) -> Self {
        Self {
            items: page.items.into_iter().map(TopicResponse::from).collect(),
            next_cursor: page.next_cursor,
        }
    }
}

#[derive(Serialize)]
struct TopicDatePageResponse {
    dates: Vec<String>,
    today: String,
    next_cursor: Option<String>,
}

impl From<TopicDatePage> for TopicDatePageResponse {
    fn from(page: TopicDatePage) -> Self {
        Self {
            dates: page.dates,
            today: page.today,
            next_cursor: page.next_cursor,
        }
    }
}

#[derive(Serialize)]
struct TopicTagResponse {
    id: Uuid,
    topic_id: Uuid,
    tag: String,
    source: &'static str,
    confidence: Option<f64>,
}

impl From<TopicTagRecord> for TopicTagResponse {
    fn from(tag: TopicTagRecord) -> Self {
        Self {
            id: tag.id,
            topic_id: tag.topic_id,
            tag: tag.tag,
            source: tag.source.as_str(),
            confidence: tag.confidence,
        }
    }
}

#[derive(Serialize)]
struct TopicTagPageResponse {
    items: Vec<TopicTagResponse>,
    next_cursor: Option<String>,
}

impl From<TopicTagPage> for TopicTagPageResponse {
    fn from(page: TopicTagPage) -> Self {
        Self {
            items: page.items.into_iter().map(TopicTagResponse::from).collect(),
            next_cursor: page.next_cursor,
        }
    }
}

#[derive(Serialize)]
struct TopicMediaPageResponse {
    items: Vec<TopicMediaResponse>,
    next_cursor: Option<String>,
}

impl From<TopicMediaPage> for TopicMediaPageResponse {
    fn from(page: TopicMediaPage) -> Self {
        Self {
            items: page
                .items
                .into_iter()
                .map(TopicMediaResponse::from)
                .collect(),
            next_cursor: page.next_cursor,
        }
    }
}

#[derive(Serialize)]
struct TopicMediaResponse {
    id: Uuid,
    topic_id: Uuid,
    media_upload_id: Uuid,
    content_type: String,
    object_key: String,
    width: Option<i32>,
    height: Option<i32>,
    byte_size: Option<i64>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

impl From<TopicMediaRecord> for TopicMediaResponse {
    fn from(media: TopicMediaRecord) -> Self {
        Self {
            id: media.id,
            topic_id: media.topic_id,
            media_upload_id: media.media_upload_id,
            content_type: media.content_type,
            object_key: media.object_key,
            width: media.width,
            height: media.height,
            byte_size: media.byte_size,
            created_at: media.created_at,
        }
    }
}
