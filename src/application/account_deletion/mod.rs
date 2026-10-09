//! Account-deletion application boundary.

pub mod cleanup;

use std::{fmt, sync::Arc};

use crate::{
    application::groups::GroupsService,
    ports::{
        account_deletion::{
            AccountDeletionCommand, AccountDeletionReport, AccountDeletionRepository,
            AccountDeletionRepositoryError, AppleReauthenticationProof,
        },
        apple_identity_provider::{
            AppleAuthorizationCodeRevocationRequest, AppleIdentityProvider,
            AppleIdentityProviderError, AppleIdentityVerificationRequest, AppleRevocationProvider,
            AppleRevocationProviderError,
        },
        push::{FenceMembershipPushCommand, PushPrivacyFence},
        transactions::{BoxTransactionHandle, TransactionManager},
    },
};

pub use crate::ports::account_deletion::ANONYMOUS_AUTHOR_NICKNAME;

#[derive(Clone)]
pub struct AccountDeletionService {
    dependencies: AccountDeletionDependencies,
}

#[derive(Clone)]
pub struct AccountDeletionDependencies {
    pub transactions: Arc<dyn TransactionManager>,
    pub groups: Arc<GroupsService>,
    pub push_privacy_fence: Arc<dyn PushPrivacyFence>,
    pub repository: Arc<dyn AccountDeletionRepository>,
    pub apple_identity_provider: Option<Arc<dyn AppleIdentityProvider>>,
    pub apple_revocation_provider: Option<Arc<dyn AppleRevocationProvider>>,
}

impl AccountDeletionService {
    pub fn new(dependencies: AccountDeletionDependencies) -> Self {
        Self { dependencies }
    }

    /// Starts one authenticated account's grace-deletion through one transaction.
    ///
    /// The repository owns the archived-group exception and returns only live
    /// memberships. Membership rows stay recoverable; Task-9 push state is
    /// fenced on the same transaction for each live membership.
    pub async fn delete_account(
        &self,
        command: AccountDeletionCommand,
    ) -> Result<AccountDeletionReport, AccountDeletionError> {
        let identity = self
            .dependencies
            .repository
            .live_identity(command.user_id)
            .await
            .map_err(AccountDeletionError::from)?;
        if let Some(identity) = identity {
            if identity.provider == "apple" {
                let proof = command
                    .apple_proof
                    .ok_or(AccountDeletionError::AppleReauthenticationRequired)?;
                validate_apple_proof(&proof)?;
                self.verify_grace_deletion_preconditions(command.user_id)
                    .await?;
                self.verify_and_revoke_apple(&identity.provider_id, proof)
                    .await?;
                return self
                    .delete_account_after_apple_revoke(command.user_id)
                    .await;
            }
            if command.apple_proof.is_some() {
                return Err(AccountDeletionError::RequestValidation);
            }
        } else if command.apple_proof.is_some() {
            return Err(AccountDeletionError::RequestValidation);
        }
        self.delete_account_grace(command.user_id).await
    }

    /// Operator path (admin CLI): starts the same grace deletion as `delete_account` for a user
    /// id after the operator verified a web deletion request.
    ///
    /// An operator cannot present the user's Sign in with Apple re-authentication, so the Apple
    /// authorization is never revoked here; `apple_authorization_not_revoked` tells the caller
    /// that the account signs in with Apple and the user has to revoke it on the device.
    pub async fn start_operator_grace_deletion(
        &self,
        user_id: uuid::Uuid,
    ) -> Result<OperatorAccountDeletion, AccountDeletionError> {
        let identity = self
            .dependencies
            .repository
            .live_identity(user_id)
            .await
            .map_err(AccountDeletionError::from)?;
        let apple_authorization_not_revoked =
            identity.is_some_and(|identity| identity.provider == "apple");
        let report = self.delete_account_grace(user_id).await?;
        tracing::info!(
            target: "jamye_server",
            event_kind = "admin_account_deletion_started",
            user_id = %user_id,
            memberships_removed = report.memberships_removed,
            apple_authorization_not_revoked,
            "operator started an account deletion"
        );
        Ok(OperatorAccountDeletion {
            report,
            apple_authorization_not_revoked,
        })
    }

    async fn verify_and_revoke_apple(
        &self,
        expected_provider_id: &str,
        proof: AppleReauthenticationProof,
    ) -> Result<(), AccountDeletionError> {
        let identity = self
            .dependencies
            .apple_identity_provider
            .as_ref()
            .ok_or(AccountDeletionError::ProviderUnavailable)?
            .verify_identity(&AppleIdentityVerificationRequest {
                identity_token: proof.identity_token,
                raw_nonce: proof.raw_nonce,
            })
            .await
            .map_err(map_apple_identity_error)?;
        if identity.provider_id != expected_provider_id {
            return Err(AccountDeletionError::AppleSubjectMismatch);
        }
        self.dependencies
            .apple_revocation_provider
            .as_ref()
            .ok_or(AccountDeletionError::ProviderUnavailable)?
            .revoke_authorization_code(&AppleAuthorizationCodeRevocationRequest {
                client_id: identity.client_id,
                authorization_code: proof.authorization_code,
            })
            .await
            .map_err(map_apple_revocation_error)
    }

    async fn verify_grace_deletion_preconditions(
        &self,
        user_id: uuid::Uuid,
    ) -> Result<(), AccountDeletionError> {
        let mut transaction = self.begin().await?;
        let result = self
            .dependencies
            .repository
            .prepare_deletion(transaction.as_mut(), user_id)
            .await
            .map(|_| ())
            .map_err(AccountDeletionError::from);
        let rollback = self.dependencies.transactions.rollback(transaction).await;
        match (result, rollback) {
            (Err(error), _) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
            (Ok(()), Err(_)) => Err(AccountDeletionError::DatabaseUnavailable),
        }
    }

    async fn delete_account_after_apple_revoke(
        &self,
        user_id: uuid::Uuid,
    ) -> Result<AccountDeletionReport, AccountDeletionError> {
        self.delete_account_grace(user_id)
            .await
            .map_err(map_post_apple_revoke_deletion_error)
    }

    async fn delete_account_grace(
        &self,
        user_id: uuid::Uuid,
    ) -> Result<AccountDeletionReport, AccountDeletionError> {
        let mut transaction = self.begin().await?;
        let preparation = match self
            .dependencies
            .repository
            .prepare_deletion(transaction.as_mut(), user_id)
            .await
        {
            Ok(preparation) => preparation,
            Err(error) => return self.finish(transaction, Err(error.into())).await,
        };

        let live_memberships_removed = match u64::try_from(preparation.memberships.len()) {
            Ok(count) => count,
            Err(_) => {
                return self
                    .finish(transaction, Err(AccountDeletionError::DatabaseUnavailable))
                    .await;
            }
        };

        let result = self
            .dependencies
            .repository
            .start_grace_period(transaction.as_mut(), user_id)
            .await
            .map_err(AccountDeletionError::from)
            .and_then(|mut report| {
                report.memberships_removed = report
                    .memberships_removed
                    .checked_add(live_memberships_removed)
                    .ok_or(AccountDeletionError::DatabaseUnavailable)?;
                Ok(report)
            });
        let report = match result {
            Ok(report) => report,
            Err(error) => return self.finish(transaction, Err(error)).await,
        };

        for membership in preparation.memberships {
            let fence = self
                .dependencies
                .push_privacy_fence
                .fence_membership_revocation(
                    transaction.as_mut(),
                    &FenceMembershipPushCommand {
                        group_id: membership.group_id,
                        user_id,
                    },
                )
                .await;
            if fence.is_err() {
                return self
                    .finish(transaction, Err(AccountDeletionError::DatabaseUnavailable))
                    .await;
            }
        }

        self.finish(transaction, Ok(report)).await
    }

    async fn begin(&self) -> Result<BoxTransactionHandle, AccountDeletionError> {
        self.dependencies
            .transactions
            .begin()
            .await
            .map_err(|_| AccountDeletionError::DatabaseUnavailable)
    }

    async fn finish<T>(
        &self,
        transaction: BoxTransactionHandle,
        result: Result<T, AccountDeletionError>,
    ) -> Result<T, AccountDeletionError> {
        match result {
            Ok(value) => {
                self.dependencies
                    .transactions
                    .commit(transaction)
                    .await
                    .map_err(|_| AccountDeletionError::DatabaseUnavailable)?;
                Ok(value)
            }
            Err(error) => {
                self.dependencies
                    .transactions
                    .rollback(transaction)
                    .await
                    .map_err(|_| AccountDeletionError::DatabaseUnavailable)?;
                Err(error)
            }
        }
    }
}

fn validate_apple_proof(proof: &AppleReauthenticationProof) -> Result<(), AccountDeletionError> {
    if proof.identity_token.is_empty()
        || proof.identity_token.len() > 8192
        || proof.identity_token.chars().any(char::is_control)
        || proof.authorization_code.is_empty()
        || proof.authorization_code.len() > 4096
        || proof.authorization_code.chars().any(char::is_control)
        || proof.raw_nonce.len() < 16
        || proof.raw_nonce.len() > 256
        || proof.raw_nonce.chars().any(char::is_control)
    {
        return Err(AccountDeletionError::RequestValidation);
    }
    Ok(())
}

fn map_apple_identity_error(error: AppleIdentityProviderError) -> AccountDeletionError {
    match error {
        AppleIdentityProviderError::Unavailable => AccountDeletionError::ProviderUnavailable,
        AppleIdentityProviderError::InvalidIdentity => {
            AccountDeletionError::AppleIdentityTokenInvalid
        }
        AppleIdentityProviderError::InvalidConfiguration => {
            AccountDeletionError::ProviderUnavailable
        }
    }
}

fn map_apple_revocation_error(error: AppleRevocationProviderError) -> AccountDeletionError {
    match error {
        AppleRevocationProviderError::Unavailable => AccountDeletionError::ProviderUnavailable,
        AppleRevocationProviderError::InvalidAuthorizationCode => {
            AccountDeletionError::AppleAuthorizationCodeInvalid
        }
        AppleRevocationProviderError::InvalidConfiguration => {
            AccountDeletionError::ProviderUnavailable
        }
    }
}

fn map_post_apple_revoke_deletion_error(error: AccountDeletionError) -> AccountDeletionError {
    match error {
        AccountDeletionError::GroupOwnershipTransferRequired => {
            AccountDeletionError::GroupOwnershipTransferRequired
        }
        _ => AccountDeletionError::DeletionFailedAfterRevoke,
    }
}

/// Result of an operator-started grace deletion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperatorAccountDeletion {
    pub report: AccountDeletionReport,
    /// The account signs in with Apple and its Apple authorization was not revoked.
    pub apple_authorization_not_revoked: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountDeletionError {
    RequestValidation,
    GroupOwnershipTransferRequired,
    AccountNotFound,
    AppleReauthenticationRequired,
    AppleIdentityTokenInvalid,
    AppleSubjectMismatch,
    AppleAuthorizationCodeInvalid,
    ProviderUnavailable,
    DeletionFailedAfterRevoke,
    DatabaseUnavailable,
}

impl From<AccountDeletionRepositoryError> for AccountDeletionError {
    fn from(error: AccountDeletionRepositoryError) -> Self {
        match error {
            AccountDeletionRepositoryError::GroupOwnershipTransferRequired => {
                Self::GroupOwnershipTransferRequired
            }
            AccountDeletionRepositoryError::AccountNotFound => Self::AccountNotFound,
            AccountDeletionRepositoryError::InvalidData
            | AccountDeletionRepositoryError::Unavailable => Self::DatabaseUnavailable,
        }
    }
}

impl fmt::Display for AccountDeletionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("account-deletion operation failed")
    }
}

impl std::error::Error for AccountDeletionError {}

#[cfg(test)]
mod tests {
    use super::{AccountDeletionError, map_post_apple_revoke_deletion_error};

    #[test]
    fn post_apple_revoke_error_mapping_keeps_group_conflict_actionable() {
        assert_eq!(
            map_post_apple_revoke_deletion_error(
                AccountDeletionError::GroupOwnershipTransferRequired
            ),
            AccountDeletionError::GroupOwnershipTransferRequired
        );
        assert_eq!(
            map_post_apple_revoke_deletion_error(AccountDeletionError::AccountNotFound),
            AccountDeletionError::DeletionFailedAfterRevoke
        );
        assert_eq!(
            map_post_apple_revoke_deletion_error(AccountDeletionError::DatabaseUnavailable),
            AccountDeletionError::DeletionFailedAfterRevoke
        );
    }
}
