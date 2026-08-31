//! Authenticated current-account deletion transport.

use std::sync::Arc;

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{FromRef, RawQuery, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::delete,
};
use uuid::Uuid;

use crate::{
    application::{
        account_deletion::{AccountDeletionError, AccountDeletionService},
        auth::AccessTokenVerifier,
    },
    ports::account_deletion::AccountDeletionCommand,
    transport::http::auth::{AuthVerifierState, AuthenticatedAccess, error_response, request_id},
};

const MAX_ACCOUNT_DELETION_BODY_BYTES: usize = 8 * 1024;

#[derive(Clone)]
pub struct AccountDeletionHttpState {
    service: Arc<AccountDeletionService>,
    verifier: AuthVerifierState,
}

impl AccountDeletionHttpState {
    pub fn new(
        service: Arc<AccountDeletionService>,
        verifier: Arc<dyn AccessTokenVerifier>,
    ) -> Self {
        Self {
            service,
            verifier: AuthVerifierState::new(verifier),
        }
    }
}

impl FromRef<AccountDeletionHttpState> for AuthVerifierState {
    fn from_ref(state: &AccountDeletionHttpState) -> Self {
        state.verifier.clone()
    }
}

pub fn router(state: AccountDeletionHttpState) -> Router {
    Router::new()
        .route("/api/v1/me", delete(delete_account))
        .with_state(state)
}

async fn delete_account(
    State(state): State<AccountDeletionHttpState>,
    AuthenticatedAccess(identity): AuthenticatedAccess,
    RawQuery(raw_query): RawQuery,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let request_id = request_id(&parts);
    if raw_query.is_some() || body_is_nonempty(body).await {
        return account_deletion_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "request_validation_failed",
            "요청 형식이 올바르지 않습니다.",
            request_id,
        );
    }

    match state
        .service
        .delete_account(AccountDeletionCommand {
            user_id: identity.user_id,
        })
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(AccountDeletionError::AccountNotFound) => account_deletion_error(
            StatusCode::UNAUTHORIZED,
            "authentication_required",
            "인증이 필요합니다.",
            request_id,
        ),
        Err(AccountDeletionError::GroupOwnershipTransferRequired) => account_deletion_error(
            StatusCode::CONFLICT,
            "group_ownership_transfer_required",
            "소유권 이양이 필요한 그룹이 있어 계정을 삭제할 수 없습니다.",
            request_id,
        ),
        Err(AccountDeletionError::DatabaseUnavailable) => account_deletion_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
            "데이터베이스를 사용할 수 없습니다.",
            request_id,
        ),
    }
}

async fn body_is_nonempty(body: Body) -> bool {
    match to_bytes(body, MAX_ACCOUNT_DELETION_BODY_BYTES).await {
        Ok(bytes) => !bytes.is_empty(),
        Err(_) => true,
    }
}

fn account_deletion_error(
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    request_id: Uuid,
) -> Response {
    tracing::warn!(
        request_id = %request_id,
        error_code = code,
        "account-deletion request rejected"
    );
    error_response(status, code, message, request_id)
}
