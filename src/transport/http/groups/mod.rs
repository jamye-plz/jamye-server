//! Authenticated Axum boundary for groups, memberships, roles, and invites.

use std::{net::SocketAddr, sync::Arc};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, FromRef, Path, RawQuery, Request, State},
    http::{HeaderValue, StatusCode, header::RETRY_AFTER},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use url::form_urlencoded;
use uuid::Uuid;

use crate::{
    application::{
        auth::AccessTokenVerifier,
        groups::{
            GroupCreateInput, GroupPatchInput, GroupsError, GroupsService, InviteCreateInput,
            PageInput,
        },
    },
    ports::groups::{
        GroupPage, GroupRecord, GroupRole, InviteJoinRecord, InviteRecord, MemberPage, MemberRecord,
    },
    transport::http::auth::{AuthVerifierState, AuthenticatedAccess, error_response, request_id},
};

const MAX_GROUP_BODY_BYTES: usize = 8 * 1024;

#[derive(Clone)]
pub struct GroupsHttpState {
    service: Arc<GroupsService>,
    verifier: AuthVerifierState,
}

impl GroupsHttpState {
    pub fn new(service: Arc<GroupsService>, verifier: Arc<dyn AccessTokenVerifier>) -> Self {
        Self {
            service,
            verifier: AuthVerifierState::new(verifier),
        }
    }
}

impl FromRef<GroupsHttpState> for AuthVerifierState {
    fn from_ref(state: &GroupsHttpState) -> Self {
        state.verifier.clone()
    }
}

pub fn router(state: GroupsHttpState) -> Router {
    Router::new()
        .route("/api/v1/groups", get(list_groups).post(create_group))
        .route(
            "/api/v1/groups/{group_id}",
            get(get_group).patch(rename_group).delete(delete_group),
        )
        .route("/api/v1/groups/{group_id}/members", get(list_members))
        .route(
            "/api/v1/groups/{group_id}/members/{user_id}",
            patch(set_member_role).delete(remove_member),
        )
        .route("/api/v1/groups/{group_id}/invites", post(create_invite))
        .route("/api/v1/invites/{code}/join", post(redeem_invite))
        .with_state(state)
}

async fn create_group(
    State(state): State<GroupsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    let result = parse_json::<GroupCreateBody>(body)
        .await
        .map(|body| GroupCreateInput { name: body.name });
    let result = match result {
        Ok(input) => state.service.create_group(identity.user_id, input).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(group) => (StatusCode::CREATED, Json(GroupResponse::from(group))).into_response(),
        Err(error) => GroupsHttpError { error, request_id }.into_response(),
    }
}

async fn list_groups(
    State(state): State<GroupsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    RawQuery(raw_query): RawQuery,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let result = parse_page(raw_query.as_deref());
    let result = match result {
        Ok(input) => state.service.list_groups(identity.user_id, input).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(page) => (StatusCode::OK, Json(GroupPageResponse::from(page))).into_response(),
        Err(error) => GroupsHttpError { error, request_id }.into_response(),
    }
}

async fn get_group(
    State(state): State<GroupsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(group_id): Path<String>,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let result = parse_uuid(&group_id);
    let result = match result {
        Ok(group_id) => state.service.get_group(identity.user_id, group_id).await,
        Err(error) => Err(error),
    };
    group_result(result, request_id, StatusCode::OK)
}

async fn list_members(
    State(state): State<GroupsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(group_id): Path<String>,
    RawQuery(raw_query): RawQuery,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let result = parse_uuid(&group_id)
        .and_then(|group_id| parse_page(raw_query.as_deref()).map(|page| (group_id, page)));
    let result = match result {
        Ok((group_id, page)) => {
            state
                .service
                .list_members(identity.user_id, group_id, page)
                .await
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(page) => (StatusCode::OK, Json(MemberPageResponse::from(page))).into_response(),
        Err(error) => GroupsHttpError { error, request_id }.into_response(),
    }
}

async fn rename_group(
    State(state): State<GroupsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(group_id): Path<String>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    let result = parse_uuid(&group_id);
    let result = match result {
        Ok(group_id) => parse_json::<GroupPatchBody>(body)
            .await
            .map(|body| (group_id, GroupPatchInput { name: body.name })),
        Err(error) => Err(error),
    };
    let result = match result {
        Ok((group_id, input)) => {
            state
                .service
                .rename_group(identity.user_id, group_id, input)
                .await
        }
        Err(error) => Err(error),
    };
    group_result(result, request_id, StatusCode::OK)
}

async fn delete_group(
    State(state): State<GroupsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(group_id): Path<String>,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let result = parse_uuid(&group_id);
    let result = match result {
        Ok(group_id) => state.service.delete_group(identity.user_id, group_id).await,
        Err(error) => Err(error),
    };
    empty_result(result, request_id)
}

async fn remove_member(
    State(state): State<GroupsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path((group_id, user_id)): Path<(String, String)>,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let result = parse_uuid(&group_id)
        .and_then(|group_id| parse_uuid(&user_id).map(|user_id| (group_id, user_id)));
    let result = match result {
        Ok((group_id, user_id)) => {
            state
                .service
                .remove_member(identity.user_id, group_id, user_id)
                .await
        }
        Err(error) => Err(error),
    };
    empty_result(result, request_id)
}

async fn set_member_role(
    State(state): State<GroupsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path((group_id, user_id)): Path<(String, String)>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    let ids = parse_uuid(&group_id)
        .and_then(|group_id| parse_uuid(&user_id).map(|user_id| (group_id, user_id)));
    let result = match ids {
        Ok((group_id, user_id)) => parse_json::<MemberRolePatchBody>(body)
            .await
            .and_then(|body| {
                GroupRole::parse(&body.role)
                    .map(|role| (group_id, user_id, role))
                    .ok_or(GroupsError::RequestValidation)
            }),
        Err(error) => Err(error),
    };
    let result = match result {
        Ok((group_id, user_id, role)) => {
            state
                .service
                .set_member_role(identity.user_id, group_id, user_id, role)
                .await
        }
        Err(error) => Err(error),
    };
    empty_result(result, request_id)
}

async fn create_invite(
    State(state): State<GroupsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(group_id): Path<String>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    let rate_limit_subject = rate_limit_subject(&parts, identity.user_id);
    let result = parse_uuid(&group_id);
    let result = match result {
        Ok(group_id) => parse_json::<InviteCreateBody>(body).await.map(|body| {
            (
                group_id,
                InviteCreateInput {
                    expires_at: body.expires_at,
                    max_uses: body.max_uses,
                },
            )
        }),
        Err(error) => Err(error),
    };
    let result = match result {
        Ok((group_id, input)) => {
            state
                .service
                .create_invite(identity.user_id, group_id, input, &rate_limit_subject)
                .await
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(invite) => (StatusCode::CREATED, Json(InviteResponse::from(invite))).into_response(),
        Err(error) => GroupsHttpError { error, request_id }.into_response(),
    }
}

async fn redeem_invite(
    State(state): State<GroupsHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    Path(code): Path<String>,
    request: Request,
) -> Response {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    let rate_limit_subject = rate_limit_subject(&parts, identity.user_id);
    match state
        .service
        .redeem_invite(identity.user_id, code, &rate_limit_subject)
        .await
    {
        Ok(result) => (StatusCode::OK, Json(InviteJoinResponse::from(result))).into_response(),
        Err(error) => GroupsHttpError { error, request_id }.into_response(),
    }
}

async fn parse_json<T>(body: Body) -> Result<T, GroupsError>
where
    T: for<'de> Deserialize<'de>,
{
    let bytes = to_bytes(body, MAX_GROUP_BODY_BYTES)
        .await
        .map_err(|_| GroupsError::RequestValidation)?;
    serde_json::from_slice(&bytes).map_err(|_| GroupsError::RequestValidation)
}

fn parse_page(raw_query: Option<&str>) -> Result<PageInput, GroupsError> {
    let mut after = None;
    let mut limit = None;
    for (key, value) in form_urlencoded::parse(raw_query.unwrap_or_default().as_bytes()) {
        match key.as_ref() {
            "after" if after.is_none() => after = Some(value.into_owned()),
            "limit" if limit.is_none() => {
                limit = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| GroupsError::RequestValidation)?,
                );
            }
            _ => return Err(GroupsError::RequestValidation),
        }
    }
    Ok(PageInput { after, limit })
}

fn parse_uuid(value: &str) -> Result<Uuid, GroupsError> {
    Uuid::try_parse(value).map_err(|_| GroupsError::RequestValidation)
}

fn group_result(
    result: Result<GroupRecord, GroupsError>,
    request_id: Uuid,
    status: StatusCode,
) -> Response {
    match result {
        Ok(group) => (status, Json(GroupResponse::from(group))).into_response(),
        Err(error) => GroupsHttpError { error, request_id }.into_response(),
    }
}

fn empty_result(result: Result<(), GroupsError>, request_id: Uuid) -> Response {
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => GroupsHttpError { error, request_id }.into_response(),
    }
}

fn rate_limit_subject(parts: &axum::http::request::Parts, user_id: Uuid) -> String {
    let address = parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(address)| address.ip().to_string())
        .unwrap_or_else(|| "unavailable".to_owned());
    format!("user:{user_id}:ip:{address}")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GroupsHttpError {
    error: GroupsError,
    request_id: Uuid,
}

impl IntoResponse for GroupsHttpError {
    fn into_response(self) -> Response {
        let (status, code, message) = error_profile(self.error);
        tracing::warn!(
            request_id = %self.request_id,
            error_code = code,
            "group request rejected"
        );
        let mut response = error_response(status, code, message, self.request_id);
        if let GroupsError::RateLimited { retry_after } = self.error {
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

fn error_profile(error: GroupsError) -> (StatusCode, &'static str, &'static str) {
    match error {
        GroupsError::RequestValidation => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "request_validation_failed",
            "요청 형식이 올바르지 않습니다.",
        ),
        GroupsError::GroupNotFound => (
            StatusCode::NOT_FOUND,
            "group_not_found",
            "그룹을 찾을 수 없습니다.",
        ),
        GroupsError::MembershipRequired => (
            StatusCode::FORBIDDEN,
            "membership_required",
            "이 그룹에 접근할 수 없습니다.",
        ),
        GroupsError::OwnerRequired => (
            StatusCode::FORBIDDEN,
            "owner_required",
            "그룹 소유자만 수행할 수 있습니다.",
        ),
        GroupsError::MemberNotFound => (
            StatusCode::NOT_FOUND,
            "member_not_found",
            "그룹 멤버를 찾을 수 없습니다.",
        ),
        GroupsError::OwnerConflict => (
            StatusCode::CONFLICT,
            "group_owner_conflict",
            "먼저 그룹 소유권을 이전해 주세요.",
        ),
        GroupsError::GroupFull => (
            StatusCode::CONFLICT,
            "group_full",
            "그룹 정원이 가득 찼습니다.",
        ),
        GroupsError::InviteNotFound => (
            StatusCode::NOT_FOUND,
            "invite_not_found",
            "초대 코드를 찾을 수 없습니다.",
        ),
        GroupsError::InviteExpired => (
            StatusCode::GONE,
            "invite_expired",
            "초대 코드가 만료되었습니다.",
        ),
        GroupsError::InviteExhausted => (
            StatusCode::GONE,
            "invite_exhausted",
            "초대 코드의 사용 횟수가 모두 소진되었습니다.",
        ),
        GroupsError::TopologyConflict => (
            StatusCode::CONFLICT,
            "group_topology_conflict",
            "그룹 기본 채팅방 구성이 충돌했습니다.",
        ),
        GroupsError::RateLimited { .. } => (
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limit_exceeded",
            "요청이 너무 많습니다. 잠시 후 다시 시도해 주세요.",
        ),
        GroupsError::RateLimitUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "rate_limit_unavailable",
            "요청 제한 서비스를 사용할 수 없습니다.",
        ),
        GroupsError::CredentialUnavailable | GroupsError::InvalidConfiguration => (
            StatusCode::SERVICE_UNAVAILABLE,
            "groups_unavailable",
            "그룹 서비스를 사용할 수 없습니다.",
        ),
        GroupsError::DatabaseUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
            "데이터베이스를 사용할 수 없습니다.",
        ),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupCreateBody {
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupPatchBody {
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberRolePatchBody {
    role: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InviteCreateBody {
    #[serde(default, with = "time::serde::rfc3339::option")]
    expires_at: Option<OffsetDateTime>,
    #[serde(default)]
    max_uses: Option<i32>,
}

#[derive(Serialize)]
struct GroupResponse {
    id: Uuid,
    name: String,
    owner_id: Uuid,
    max_members: i32,
    member_count: i64,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    main_chatroom_id: Uuid,
}

impl From<GroupRecord> for GroupResponse {
    fn from(group: GroupRecord) -> Self {
        Self {
            id: group.id,
            name: group.name,
            owner_id: group.owner_id,
            max_members: group.max_members,
            member_count: group.member_count,
            created_at: group.created_at,
            main_chatroom_id: group.main_chatroom_id,
        }
    }
}

#[derive(Serialize)]
struct GroupPageResponse {
    items: Vec<GroupResponse>,
    next_cursor: Option<String>,
}

impl From<GroupPage> for GroupPageResponse {
    fn from(page: GroupPage) -> Self {
        Self {
            items: page.items.into_iter().map(GroupResponse::from).collect(),
            next_cursor: page.next_cursor,
        }
    }
}

#[derive(Serialize)]
struct MemberResponse {
    user_id: Uuid,
    nickname: String,
    avatar_url: Option<String>,
    role: &'static str,
    #[serde(with = "time::serde::rfc3339")]
    joined_at: OffsetDateTime,
}

impl From<MemberRecord> for MemberResponse {
    fn from(member: MemberRecord) -> Self {
        Self {
            user_id: member.user_id,
            nickname: member.nickname,
            avatar_url: member.avatar_url,
            role: member.role.as_str(),
            joined_at: member.joined_at,
        }
    }
}

#[derive(Serialize)]
struct MemberPageResponse {
    items: Vec<MemberResponse>,
    next_cursor: Option<String>,
}

impl From<MemberPage> for MemberPageResponse {
    fn from(page: MemberPage) -> Self {
        Self {
            items: page.items.into_iter().map(MemberResponse::from).collect(),
            next_cursor: page.next_cursor,
        }
    }
}

#[derive(Serialize)]
struct InviteResponse {
    id: Uuid,
    group_id: Uuid,
    code: String,
    created_by: Uuid,
    #[serde(with = "time::serde::rfc3339::option")]
    expires_at: Option<OffsetDateTime>,
    max_uses: Option<i32>,
    used_count: i32,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

impl From<InviteRecord> for InviteResponse {
    fn from(invite: InviteRecord) -> Self {
        Self {
            id: invite.id,
            group_id: invite.group_id,
            code: invite.code,
            created_by: invite.created_by,
            expires_at: invite.expires_at,
            max_uses: invite.max_uses,
            used_count: invite.used_count,
            created_at: invite.created_at,
        }
    }
}

#[derive(Serialize)]
struct InviteJoinResponse {
    group_id: Uuid,
    membership_id: Option<Uuid>,
    joined: bool,
}

impl From<InviteJoinRecord> for InviteJoinResponse {
    fn from(result: InviteJoinRecord) -> Self {
        Self {
            group_id: result.group_id,
            membership_id: result.membership_id,
            joined: result.joined,
        }
    }
}
