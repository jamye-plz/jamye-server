//! Bearer extraction and stable authentication failures.

mod api;

pub use api::{AuthHttpState, router};

use std::sync::Arc;

use axum::{
    Json,
    extract::{FromRef, FromRequestParts},
    http::{StatusCode, header::AUTHORIZATION, request::Parts},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use serde_json::Value;
use tower_http::request_id::RequestId;
use uuid::Uuid;

use crate::application::auth::{
    AccessAccountGate, AccessGateError, AccessIdentity, AccessTokenVerifier,
};

const AUTHENTICATION_CODE: &str = "authentication_required";
const AUTHENTICATION_MESSAGE: &str = "인증이 필요합니다.";
const SUSPENDED_CODE: &str = "account_suspended";
const SUSPENDED_MESSAGE: &str = "정지된 계정입니다.";
const UNAVAILABLE_CODE: &str = "database_unavailable";
const UNAVAILABLE_MESSAGE: &str = "데이터베이스를 사용할 수 없습니다.";

#[derive(Clone)]
pub struct AuthVerifierState {
    verifier: Arc<dyn AccessTokenVerifier>,
}

impl AuthVerifierState {
    pub fn new(verifier: Arc<dyn AccessTokenVerifier>) -> Self {
        Self { verifier }
    }

    /// The account gate (suspension) of the configured verifier, when it has one.
    pub(crate) fn account_gate(&self) -> Option<Arc<dyn AccessAccountGate>> {
        self.verifier.account_gate()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedAccess(pub AccessIdentity);

impl<S> FromRequestParts<S> for AuthenticatedAccess
where
    S: Send + Sync,
    AuthVerifierState: FromRef<S>,
{
    type Rejection = AuthenticationRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let request_id = request_id(parts);
        let unauthenticated = AuthenticationRejection {
            request_id,
            kind: RejectionKind::Unauthenticated,
        };
        let token = bearer_token(parts).ok_or(unauthenticated)?;
        let verifier = AuthVerifierState::from_ref(state);
        let identity = verifier
            .verifier
            .verify(token)
            .map_err(|_| unauthenticated)?;
        if let Some(gate) = verifier.verifier.account_gate() {
            match gate.check(identity.user_id).await {
                Ok(()) => {}
                Err(AccessGateError::Suspended) => {
                    return Err(AuthenticationRejection {
                        request_id,
                        kind: RejectionKind::Suspended,
                    });
                }
                Err(AccessGateError::Unavailable) => {
                    return Err(AuthenticationRejection {
                        request_id,
                        kind: RejectionKind::Unavailable,
                    });
                }
            }
        }
        Ok(Self(identity))
    }
}

fn bearer_token(parts: &Parts) -> Option<&str> {
    let mut values = parts.headers.get_all(AUTHORIZATION).iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }

    let value = value.to_str().ok()?;
    let token = value.strip_prefix("Bearer ")?;
    if token.is_empty() || token.chars().any(char::is_whitespace) {
        return None;
    }
    Some(token)
}

pub(crate) fn request_id(parts: &Parts) -> Uuid {
    parts
        .extensions
        .get::<RequestId>()
        .and_then(|value| value.header_value().to_str().ok())
        .and_then(|value| Uuid::try_parse(value).ok())
        .unwrap_or_else(Uuid::new_v4)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RejectionKind {
    Unauthenticated,
    Suspended,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthenticationRejection {
    request_id: Uuid,
    kind: RejectionKind,
}

impl IntoResponse for AuthenticationRejection {
    fn into_response(self) -> Response {
        let (status, code, message) = match self.kind {
            RejectionKind::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                AUTHENTICATION_CODE,
                AUTHENTICATION_MESSAGE,
            ),
            RejectionKind::Suspended => (StatusCode::FORBIDDEN, SUSPENDED_CODE, SUSPENDED_MESSAGE),
            RejectionKind::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                UNAVAILABLE_CODE,
                UNAVAILABLE_MESSAGE,
            ),
        };
        error_response(status, code, message, self.request_id)
    }
}

pub(crate) fn error_response(
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    request_id: Uuid,
) -> Response {
    (
        status,
        Json(ErrorEnvelope {
            error: ErrorBody {
                code,
                message,
                request_id,
                details: None,
            },
        }),
    )
        .into_response()
}

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: &'static str,
    request_id: Uuid,
    details: Option<Value>,
}
