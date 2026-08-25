//! R1 ticket issuance and one-time-ticket WebSocket upgrade boundary.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{FromRef, RawQuery, State, ws::WebSocketUpgrade},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use url::form_urlencoded;
use uuid::Uuid;

use crate::{
    application::realtime::{RealtimeTicketError, RealtimeTicketService},
    ports::realtime::ConversationAuthorizer,
    transport::{
        http::auth::{AuthVerifierState, AuthenticatedAccess, error_response},
        realtime::{LocalRealtimeHub, run_socket, run_unauthenticated_socket},
    },
};

const CONTRACT_VERSION_HEADER: &str = "x-jamye-contract-version";

#[derive(Clone)]
pub struct RealtimeHttpState {
    tickets: Arc<RealtimeTicketService>,
    hub: LocalRealtimeHub,
    authorizer: Arc<dyn ConversationAuthorizer>,
    auth: AuthVerifierState,
}

impl RealtimeHttpState {
    pub fn new(
        tickets: Arc<RealtimeTicketService>,
        hub: LocalRealtimeHub,
        authorizer: Arc<dyn ConversationAuthorizer>,
        auth: AuthVerifierState,
    ) -> Self {
        Self {
            tickets,
            hub,
            authorizer,
            auth,
        }
    }
}

impl FromRef<RealtimeHttpState> for AuthVerifierState {
    fn from_ref(state: &RealtimeHttpState) -> Self {
        state.auth.clone()
    }
}

pub fn router(state: RealtimeHttpState) -> Router {
    Router::new()
        .route("/api/v1/realtime/tickets", post(issue_ticket))
        .route("/api/v1/realtime/ws", get(websocket))
        .with_state(state)
}

async fn issue_ticket(
    State(state): State<RealtimeHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    headers: HeaderMap,
) -> Response {
    let request_id = Uuid::new_v4();
    let version = match required_contract_version(&headers) {
        Ok(version) => version,
        Err(error) => return ticket_error(error, request_id),
    };
    match state.tickets.issue(&identity, &version).await {
        Ok(ticket) => {
            let mut response = (
                StatusCode::CREATED,
                Json(TicketResponse {
                    ticket: ticket.ticket,
                    expires_at: ticket.expires_at,
                    contract_version: ticket.contract_version.clone(),
                }),
            )
                .into_response();
            if let Ok(value) = HeaderValue::from_str(&ticket.contract_version) {
                response
                    .headers_mut()
                    .insert(CONTRACT_VERSION_HEADER, value);
            }
            tracing::info!(
                request_id = %request_id,
                user_id = %identity.user_id,
                contract_version = %ticket.contract_version,
                "realtime ticket issued"
            );
            response
        }
        Err(error) => ticket_error(error, request_id),
    }
}

async fn websocket(
    State(state): State<RealtimeHttpState>,
    ws: WebSocketUpgrade,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let raw_ticket = match parse_ticket_query(raw_query.as_deref()) {
        Some(ticket) => ticket,
        None => return ws.on_upgrade(run_unauthenticated_socket),
    };
    match state.tickets.consume(&raw_ticket).await {
        Ok(session) => {
            let hub = state.hub.clone();
            let authorizer = state.authorizer.clone();
            ws.on_upgrade(move |socket| run_socket(socket, session, hub, authorizer))
        }
        Err(RealtimeTicketError::Unavailable) => {
            ticket_error(RealtimeTicketError::Unavailable, Uuid::new_v4())
        }
        Err(_) => ws.on_upgrade(run_unauthenticated_socket),
    }
}

fn required_contract_version(headers: &HeaderMap) -> Result<String, RealtimeTicketError> {
    let mut values = headers.get_all(CONTRACT_VERSION_HEADER).iter();
    let value = values
        .next()
        .ok_or(RealtimeTicketError::ContractUpgradeRequired)?;
    if values.next().is_some() {
        return Err(RealtimeTicketError::ContractUpgradeRequired);
    }
    value
        .to_str()
        .map(str::to_owned)
        .map_err(|_| RealtimeTicketError::ContractUpgradeRequired)
}

fn parse_ticket_query(raw_query: Option<&str>) -> Option<String> {
    let mut ticket = None;
    for (key, value) in form_urlencoded::parse(raw_query.unwrap_or_default().as_bytes()) {
        if key != "ticket" || ticket.is_some() {
            return None;
        }
        ticket = Some(value.into_owned());
    }
    ticket.filter(|ticket| !ticket.is_empty() && !ticket.chars().any(char::is_whitespace))
}

fn ticket_error(error: RealtimeTicketError, request_id: Uuid) -> Response {
    let (status, code, message) = match error {
        RealtimeTicketError::AuthenticationRequired | RealtimeTicketError::AuthenticationFailed => {
            (
                StatusCode::UNAUTHORIZED,
                "authentication_required",
                "인증이 필요합니다.",
            )
        }
        RealtimeTicketError::ContractUpgradeRequired => (
            StatusCode::UPGRADE_REQUIRED,
            "contract_upgrade_required",
            "지원되는 계약 버전으로 앱을 업데이트해 주세요.",
        ),
        RealtimeTicketError::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "realtime_unavailable",
            "실시간 연결을 사용할 수 없습니다.",
        ),
    };
    tracing::warn!(
        request_id = %request_id,
        error_code = code,
        "realtime request rejected"
    );
    error_response(status, code, message, request_id)
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct TicketResponse {
    ticket: String,
    #[serde(with = "time::serde::rfc3339")]
    expires_at: time::OffsetDateTime,
    contract_version: String,
}
