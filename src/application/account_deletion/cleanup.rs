//! Durable account-object cleanup worker boundary.
//!
//! One run claims a bounded batch, deletes every claimed object concurrently,
//! and persists only generation-fenced terminal or retry transitions.

use std::{fmt, sync::Arc, time::Duration};

use futures_util::future::join_all;

use crate::ports::account_deletion::{
    AccountObjectDeletionClaim, AccountObjectDeletionClaimRequest,
    AccountObjectDeletionFailureCode, AccountObjectDeletionFailureDisposition,
    AccountObjectDeletionProvider, AccountObjectDeletionRepository,
};
use crate::ports::object_storage::ObjectStorageProviderError;

const MAX_CLAIM_OWNER_CHARS: usize = 128;

#[derive(Clone)]
pub struct AccountObjectDeletionWorker {
    dependencies: AccountObjectDeletionWorkerDependencies,
    config: AccountObjectDeletionWorkerConfig,
}

#[derive(Clone)]
pub struct AccountObjectDeletionWorkerDependencies {
    pub repository: Arc<dyn AccountObjectDeletionRepository>,
    pub provider: Arc<dyn AccountObjectDeletionProvider>,
}

impl AccountObjectDeletionWorker {
    pub fn new(
        dependencies: AccountObjectDeletionWorkerDependencies,
        config: AccountObjectDeletionWorkerConfig,
    ) -> Result<Self, AccountObjectDeletionWorkerError> {
        config.validate()?;
        Ok(Self {
            dependencies,
            config,
        })
    }

    /// Claims, deletes, and durably records one bounded batch.
    pub async fn run_once(
        &self,
    ) -> Result<AccountObjectDeletionWorkerReport, AccountObjectDeletionWorkerError> {
        let claims = self
            .dependencies
            .repository
            .claim_object_deletions(AccountObjectDeletionClaimRequest {
                claim_owner: self.config.claim_owner.clone(),
                batch_size: self.config.batch_size,
                lease_duration: self.config.lease_duration,
            })
            .await
            .map_err(|_| AccountObjectDeletionWorkerError::RepositoryUnavailable)?;
        let mut report = AccountObjectDeletionWorkerReport {
            claimed: claims.len(),
            ..AccountObjectDeletionWorkerReport::default()
        };
        let outcomes = join_all(claims.into_iter().map(|claim| self.process_claim(claim))).await;
        for outcome in outcomes {
            report.merge(outcome?);
        }
        Ok(report)
    }

    async fn process_claim(
        &self,
        claim: AccountObjectDeletionClaim,
    ) -> Result<AccountObjectDeletionWorkerReport, AccountObjectDeletionWorkerError> {
        let mut report = AccountObjectDeletionWorkerReport::default();
        match tokio::time::timeout(
            self.config.delete_timeout,
            self.dependencies.provider.delete_object(claim.object_key()),
        )
        .await
        {
            Ok(Ok(())) => {
                if self
                    .dependencies
                    .repository
                    .mark_object_deleted(&claim)
                    .await
                    .map_err(|_| AccountObjectDeletionWorkerError::RepositoryUnavailable)?
                {
                    report.succeeded = 1;
                } else {
                    report.stale_claims = 1;
                }
            }
            Ok(Err(error)) => {
                self.record_failure(&claim, failure_code(error), &mut report)
                    .await?;
            }
            Err(_) => {
                self.record_failure(
                    &claim,
                    AccountObjectDeletionFailureCode::Timeout,
                    &mut report,
                )
                .await?;
            }
        }
        Ok(report)
    }

    async fn record_failure(
        &self,
        claim: &AccountObjectDeletionClaim,
        code: AccountObjectDeletionFailureCode,
        report: &mut AccountObjectDeletionWorkerReport,
    ) -> Result<(), AccountObjectDeletionWorkerError> {
        match self
            .dependencies
            .repository
            .record_object_deletion_failure(
                claim,
                code,
                self.config.retry_delay,
                self.config.max_attempts,
            )
            .await
            .map_err(|_| AccountObjectDeletionWorkerError::RepositoryUnavailable)?
        {
            AccountObjectDeletionFailureDisposition::RetryScheduled => report.retries += 1,
            AccountObjectDeletionFailureDisposition::Failed => report.failed += 1,
            AccountObjectDeletionFailureDisposition::DeadLettered => {
                report.dead_lettered += 1;
                tracing::error!(
                    cleanup_intent_id = %claim.intent_id,
                    failure_code = code.as_str(),
                    "account object deletion reached the terminal dead-letter state"
                );
            }
            AccountObjectDeletionFailureDisposition::StaleClaim => report.stale_claims += 1,
        }
        Ok(())
    }

    pub fn poll_interval(&self) -> Duration {
        self.config.poll_interval
    }
}

fn failure_code(error: ObjectStorageProviderError) -> AccountObjectDeletionFailureCode {
    match error {
        ObjectStorageProviderError::AccessDenied => AccountObjectDeletionFailureCode::AccessDenied,
        ObjectStorageProviderError::Unavailable => AccountObjectDeletionFailureCode::Unavailable,
        ObjectStorageProviderError::UnexpectedResponse => {
            AccountObjectDeletionFailureCode::UnexpectedResponse
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountObjectDeletionWorkerConfig {
    pub claim_owner: String,
    pub batch_size: u32,
    pub lease_duration: Duration,
    pub delete_timeout: Duration,
    pub lease_safety_margin: Duration,
    pub retry_delay: Duration,
    pub poll_interval: Duration,
    pub max_attempts: u32,
}

impl AccountObjectDeletionWorkerConfig {
    fn validate(&self) -> Result<(), AccountObjectDeletionWorkerError> {
        let delete_budget = self
            .delete_timeout
            .checked_add(self.lease_safety_margin)
            .ok_or(AccountObjectDeletionWorkerError::InvalidConfiguration)?;
        let owner_length = self.claim_owner.chars().count();
        if owner_length == 0
            || owner_length > MAX_CLAIM_OWNER_CHARS
            || self.claim_owner.trim() != self.claim_owner
            || self.claim_owner.chars().any(char::is_control)
            || self.batch_size == 0
            || self.max_attempts == 0
            || self.lease_duration.is_zero()
            || self.delete_timeout.is_zero()
            || self.lease_safety_margin.is_zero()
            || self.lease_duration <= delete_budget
            || self.retry_delay.is_zero()
            || self.poll_interval.is_zero()
        {
            return Err(AccountObjectDeletionWorkerError::InvalidConfiguration);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AccountObjectDeletionWorkerReport {
    pub claimed: usize,
    pub succeeded: usize,
    pub retries: usize,
    pub failed: usize,
    pub dead_lettered: usize,
    pub stale_claims: usize,
}

impl AccountObjectDeletionWorkerReport {
    fn merge(&mut self, other: Self) {
        self.succeeded += other.succeeded;
        self.retries += other.retries;
        self.failed += other.failed;
        self.dead_lettered += other.dead_lettered;
        self.stale_claims += other.stale_claims;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountObjectDeletionWorkerError {
    InvalidConfiguration,
    RepositoryUnavailable,
}

impl fmt::Display for AccountObjectDeletionWorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("account object-deletion worker operation failed")
    }
}

impl std::error::Error for AccountObjectDeletionWorkerError {}
