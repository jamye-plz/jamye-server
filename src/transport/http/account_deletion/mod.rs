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
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    application::{
        account_deletion::{AccountDeletionError, AccountDeletionService},
        auth::AccessTokenVerifier,
    },
    ports::account_deletion::{AccountDeletionCommand, AppleReauthenticationProof},
    transport::http::auth::{AuthVerifierState, AuthenticatedAccess, error_response, request_id},
};

const MAX_ACCOUNT_DELETION_BODY_BYTES: usize = 16 * 1024;

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
    if raw_query.is_some() {
        return account_deletion_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "request_validation_failed",
            "요청 형식이 올바르지 않습니다.",
            request_id,
        );
    }
    let proof = match parse_optional_apple_proof(body).await {
        Ok(proof) => proof,
        Err(()) => {
            return account_deletion_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "request_validation_failed",
                "요청 형식이 올바르지 않습니다.",
                request_id,
            );
        }
    };

    match state
        .service
        .delete_account(AccountDeletionCommand {
            user_id: identity.user_id,
            apple_proof: proof,
        })
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(AccountDeletionError::RequestValidation) => account_deletion_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "request_validation_failed",
            "요청 형식이 올바르지 않습니다.",
            request_id,
        ),
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
        Err(AccountDeletionError::AppleReauthenticationRequired) => account_deletion_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "apple_reauthentication_required",
            "Apple 인증을 다시 진행해야 합니다.",
            request_id,
        ),
        Err(AccountDeletionError::AppleIdentityTokenInvalid) => account_deletion_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "apple_identity_token_invalid",
            "Apple 인증 정보를 확인할 수 없습니다.",
            request_id,
        ),
        Err(AccountDeletionError::AppleSubjectMismatch) => account_deletion_error(
            StatusCode::FORBIDDEN,
            "apple_subject_mismatch",
            "다른 Apple 계정으로는 삭제할 수 없습니다.",
            request_id,
        ),
        Err(AccountDeletionError::AppleAuthorizationCodeInvalid) => account_deletion_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "apple_authorization_code_invalid",
            "Apple 인증 코드를 확인할 수 없습니다.",
            request_id,
        ),
        Err(AccountDeletionError::ProviderUnavailable) => account_deletion_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "provider_unavailable",
            "로그인 제공자를 일시적으로 사용할 수 없습니다.",
            request_id,
        ),
        Err(AccountDeletionError::DeletionFailedAfterRevoke) => account_deletion_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "account_deletion_failed_after_revoke",
            "Apple 연결 폐기 뒤 계정 삭제를 완료하지 못했습니다.",
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

async fn parse_optional_apple_proof(body: Body) -> Result<Option<AppleReauthenticationProof>, ()> {
    let bytes = to_bytes(body, MAX_ACCOUNT_DELETION_BODY_BYTES)
        .await
        .map_err(|_| ())?;
    if bytes.is_empty() {
        return Ok(None);
    }
    let body = serde_json::from_slice::<AppleProofBody>(&bytes).map_err(|_| ())?;
    Ok(Some(AppleReauthenticationProof {
        identity_token: body.identity_token,
        authorization_code: body.authorization_code,
        raw_nonce: body.raw_nonce,
    }))
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppleProofBody {
    identity_token: String,
    authorization_code: String,
    raw_nonce: String,
}
