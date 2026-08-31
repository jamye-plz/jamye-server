//! Account-deletion application boundary.

pub mod cleanup;

use std::{fmt, sync::Arc};

use crate::{
    application::groups::GroupsService,
    ports::{
        account_deletion::{
            AccountDeletionCommand, AccountDeletionReport, AccountDeletionRepository,
            AccountDeletionRepositoryError,
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
}

impl AccountDeletionService {
    pub fn new(dependencies: AccountDeletionDependencies) -> Self {
        Self { dependencies }
    }

    /// Deletes one authenticated account through its sole caller-owned transaction.
    ///
    /// The repository owns the archived-group exception and returns only live
    /// memberships. Each live membership is removed through the Task-6 boundary
    /// and immediately fences its Task-9 push state on the same transaction.
    pub async fn delete_account(
        &self,
        command: AccountDeletionCommand,
    ) -> Result<AccountDeletionReport, AccountDeletionError> {
        let mut transaction = self.begin().await?;
        let preparation = match self
            .dependencies
            .repository
            .prepare_deletion(transaction.as_mut(), command.user_id)
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

        for membership in preparation.memberships {
            let removal = self
                .dependencies
                .groups
                .remove_member_in_transaction(
                    transaction.as_mut(),
                    command.user_id,
                    membership.group_id,
                    command.user_id,
                )
                .await;
            if removal.is_err() {
                return self
                    .finish(transaction, Err(AccountDeletionError::DatabaseUnavailable))
                    .await;
            }

            let fence = self
                .dependencies
                .push_privacy_fence
                .fence_membership_revocation(
                    transaction.as_mut(),
                    &FenceMembershipPushCommand {
                        group_id: membership.group_id,
                        user_id: command.user_id,
                    },
                )
                .await;
            if fence.is_err() {
                return self
                    .finish(transaction, Err(AccountDeletionError::DatabaseUnavailable))
                    .await;
            }
        }

        let result = self
            .dependencies
            .repository
            .finalize_deletion(transaction.as_mut(), command.user_id)
            .await
            .map_err(AccountDeletionError::from)
            .and_then(|mut report| {
                report.memberships_removed = report
                    .memberships_removed
                    .checked_add(live_memberships_removed)
                    .ok_or(AccountDeletionError::DatabaseUnavailable)?;
                Ok(report)
            });
        self.finish(transaction, result).await
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountDeletionError {
    GroupOwnershipTransferRequired,
    AccountNotFound,
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
