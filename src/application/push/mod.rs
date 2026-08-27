//! Expo installation registration, update, and owner-scoped deletion use cases.

use std::{fmt, sync::Arc, time::Duration};

use futures_util::future::join_all;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::ports::{
    push::{
        AuthorizedPushDelivery, ClaimedPushDelivery, DeletePushInstallationCommand,
        PushDeliveryClaimRequest, PushDeliveryFailureCode, PushDeliveryFailureDisposition,
        PushEnvironment, PushInstallationRecord, PushPlatform, PushPreviewSource, PushProvider,
        PushProviderError, PushProviderName, PushProviderOutcome, PushProviderRequest,
        PushRepository, PushRepositoryError, PushWorkerRepository, UpdatePushInstallationCommand,
        UpsertPushInstallationCommand,
    },
    transactions::{BoxTransactionHandle, TransactionManager},
};

const MAX_INSTALLATION_ID_CHARS: usize = 255;
const MAX_EXPO_TOKEN_CHARS: usize = 512;
const MAX_PUSH_CLAIM_OWNER_CHARS: usize = 128;

#[derive(Clone)]
pub struct PushService {
    dependencies: PushDependencies,
}

#[derive(Clone)]
pub struct PushDependencies {
    pub transactions: Arc<dyn TransactionManager>,
    pub repository: Arc<dyn PushRepository>,
}

impl PushService {
    pub fn new(dependencies: PushDependencies) -> Self {
        Self { dependencies }
    }

    pub async fn upsert_installation(
        &self,
        user_id: Uuid,
        input: ExpoInstallationCreateInput,
    ) -> Result<PushInstallationUpsert, PushError> {
        let platform = PushPlatform::parse(&input.platform).ok_or(PushError::RequestValidation)?;
        let environment =
            PushEnvironment::parse(&input.environment).ok_or(PushError::RequestValidation)?;
        validate_bounded(&input.installation_id, MAX_INSTALLATION_ID_CHARS)?;
        validate_bounded(&input.expo_token, MAX_EXPO_TOKEN_CHARS)?;
        let command = UpsertPushInstallationCommand {
            id: Uuid::new_v4(),
            user_id,
            installation_id: input.installation_id,
            platform,
            provider: PushProviderName::Expo,
            token: input.expo_token,
            environment,
            message_preview_enabled: input.message_preview_enabled.unwrap_or(false),
        };
        let mut transaction = self.begin().await?;
        let result = self
            .dependencies
            .repository
            .upsert_installation(transaction.as_mut(), &command)
            .await
            .map(|outcome| PushInstallationUpsert {
                installation: public_installation(outcome.installation),
                created: outcome.created,
            })
            .map_err(PushError::from);
        self.finish(transaction, result).await
    }

    pub async fn update_installation(
        &self,
        user_id: Uuid,
        installation_id: String,
        input: ExpoInstallationPutInput,
    ) -> Result<PushInstallation, PushError> {
        validate_bounded(&installation_id, MAX_INSTALLATION_ID_CHARS)?;
        validate_bounded(&input.expo_token, MAX_EXPO_TOKEN_CHARS)?;
        let command = UpdatePushInstallationCommand {
            user_id,
            installation_id,
            token: input.expo_token,
            message_preview_enabled: input.message_preview_enabled,
        };
        let mut transaction = self.begin().await?;
        let result = self
            .dependencies
            .repository
            .update_installation(transaction.as_mut(), &command)
            .await
            .map(public_installation)
            .map_err(PushError::from);
        self.finish(transaction, result).await
    }

    pub async fn delete_installation(
        &self,
        user_id: Uuid,
        installation_id: String,
    ) -> Result<(), PushError> {
        validate_bounded(&installation_id, MAX_INSTALLATION_ID_CHARS)?;
        let command = DeletePushInstallationCommand {
            user_id,
            installation_id,
        };
        let mut transaction = self.begin().await?;
        let result = self
            .dependencies
            .repository
            .delete_installation(transaction.as_mut(), &command)
            .await
            .map_err(PushError::from);
        self.finish(transaction, result).await
    }

    async fn begin(&self) -> Result<BoxTransactionHandle, PushError> {
        self.dependencies
            .transactions
            .begin()
            .await
            .map_err(|_| PushError::DatabaseUnavailable)
    }

    async fn finish<T>(
        &self,
        transaction: BoxTransactionHandle,
        result: Result<T, PushError>,
    ) -> Result<T, PushError> {
        match result {
            Ok(value) => {
                self.dependencies
                    .transactions
                    .commit(transaction)
                    .await
                    .map_err(|_| PushError::DatabaseUnavailable)?;
                Ok(value)
            }
            Err(error) => {
                self.dependencies
                    .transactions
                    .rollback(transaction)
                    .await
                    .map_err(|_| PushError::DatabaseUnavailable)?;
                Err(error)
            }
        }
    }
}

#[derive(Clone)]
pub struct PushWorker {
    dependencies: PushWorkerDependencies,
    config: PushWorkerConfig,
}

#[derive(Clone)]
pub struct PushWorkerDependencies {
    pub transactions: Arc<dyn TransactionManager>,
    pub repository: Arc<dyn PushWorkerRepository>,
    pub preview_source: Arc<dyn PushPreviewSource>,
    pub provider: Arc<dyn PushProvider>,
}

impl PushWorker {
    pub fn new(
        dependencies: PushWorkerDependencies,
        config: PushWorkerConfig,
    ) -> Result<Self, PushWorkerError> {
        config.validate()?;
        Ok(Self {
            dependencies,
            config,
        })
    }

    pub async fn run_once(&self) -> Result<PushWorkerReport, PushWorkerError> {
        let claims = self
            .dependencies
            .repository
            .claim_deliveries(PushDeliveryClaimRequest {
                claim_owner: self.config.claim_owner.clone(),
                batch_size: self.config.batch_size,
                lease_duration: self.config.lease_duration,
            })
            .await
            .map_err(|_| PushWorkerError::RepositoryUnavailable)?;
        let mut report = PushWorkerReport {
            claimed: claims.len(),
            ..PushWorkerReport::default()
        };
        let outcomes = join_all(claims.into_iter().map(|claim| self.process_claim(claim))).await;
        for outcome in outcomes {
            report.merge(outcome?);
        }
        Ok(report)
    }

    pub fn poll_interval(&self) -> Duration {
        self.config.poll_interval
    }

    async fn process_claim(
        &self,
        claim: ClaimedPushDelivery,
    ) -> Result<PushWorkerReport, PushWorkerError> {
        let mut report = PushWorkerReport::default();
        let Some(authorized) = self.authorize(&claim).await? else {
            report.authorization_denied = 1;
            self.record_failure(
                &claim,
                PushDeliveryFailureCode::AuthorizationDenied,
                1,
                &mut report,
            )
            .await?;
            return Ok(report);
        };
        let preview = match self.load_preview(&authorized).await {
            Ok(preview) => preview,
            Err(_) => {
                self.record_failure(
                    &claim,
                    PushDeliveryFailureCode::PreviewUnavailable,
                    self.config.max_attempts,
                    &mut report,
                )
                .await?;
                return Ok(report);
            }
        };
        let request = PushProviderRequest {
            destination: authorized.destination,
            route: authorized.route,
            preview,
        };
        match tokio::time::timeout(
            self.config.provider_timeout,
            self.dependencies.provider.send(&request),
        )
        .await
        {
            Ok(Ok(PushProviderOutcome::Accepted)) => {
                if self
                    .dependencies
                    .repository
                    .mark_delivery_succeeded(&claim)
                    .await
                    .map_err(|_| PushWorkerError::RepositoryUnavailable)?
                {
                    report.succeeded = 1;
                } else {
                    report.stale_claims = 1;
                }
            }
            Ok(Ok(PushProviderOutcome::DeviceNotRegistered)) => {
                self.disable_invalid_destination(&claim, &request, &mut report)
                    .await?;
            }
            Ok(Err(PushProviderError::Unavailable)) => {
                self.record_failure(
                    &claim,
                    PushDeliveryFailureCode::ExpoUnavailable,
                    self.config.max_attempts,
                    &mut report,
                )
                .await?;
            }
            Ok(Err(PushProviderError::Rejected)) => {
                self.record_failure(
                    &claim,
                    PushDeliveryFailureCode::ExpoRejected,
                    1,
                    &mut report,
                )
                .await?;
            }
            Err(_) => {
                self.record_failure(
                    &claim,
                    PushDeliveryFailureCode::ExpoTimeout,
                    self.config.max_attempts,
                    &mut report,
                )
                .await?;
            }
        }
        Ok(report)
    }

    async fn authorize(
        &self,
        claim: &ClaimedPushDelivery,
    ) -> Result<Option<AuthorizedPushDelivery>, PushWorkerError> {
        let mut transaction = self
            .dependencies
            .transactions
            .begin()
            .await
            .map_err(|_| PushWorkerError::RepositoryUnavailable)?;
        let result = self
            .dependencies
            .repository
            .authorize_send(transaction.as_mut(), &claim.claim)
            .await;
        match result {
            Ok(Some(authorized)) if authorized.occurrence_id == claim.claim.occurrence_id => {
                self.dependencies
                    .transactions
                    .commit(transaction)
                    .await
                    .map_err(|_| PushWorkerError::RepositoryUnavailable)?;
                Ok(Some(authorized))
            }
            Ok(None) => {
                self.dependencies
                    .transactions
                    .rollback(transaction)
                    .await
                    .map_err(|_| PushWorkerError::RepositoryUnavailable)?;
                Ok(None)
            }
            Ok(Some(_)) | Err(_) => {
                self.dependencies
                    .transactions
                    .rollback(transaction)
                    .await
                    .map_err(|_| PushWorkerError::RepositoryUnavailable)?;
                Err(PushWorkerError::RepositoryUnavailable)
            }
        }
    }

    async fn load_preview(
        &self,
        authorized: &AuthorizedPushDelivery,
    ) -> Result<Option<String>, PushRepositoryError> {
        let Some(message_id) = authorized.preview_message_id else {
            return Ok(None);
        };
        self.dependencies
            .preview_source
            .load_message_body(message_id)
            .await
            .map(|body| body.and_then(|body| normalized_preview(&body)))
    }

    async fn disable_invalid_destination(
        &self,
        claim: &ClaimedPushDelivery,
        request: &PushProviderRequest,
        report: &mut PushWorkerReport,
    ) -> Result<(), PushWorkerError> {
        let mut transaction = self
            .dependencies
            .transactions
            .begin()
            .await
            .map_err(|_| PushWorkerError::RepositoryUnavailable)?;
        let result = self
            .dependencies
            .repository
            .disable_invalid_destination(transaction.as_mut(), &claim.claim, &request.destination)
            .await;
        match result {
            Ok(true) => {
                self.dependencies
                    .transactions
                    .commit(transaction)
                    .await
                    .map_err(|_| PushWorkerError::RepositoryUnavailable)?;
                report.invalid_destinations = 1;
            }
            Ok(false) => {
                self.dependencies
                    .transactions
                    .rollback(transaction)
                    .await
                    .map_err(|_| PushWorkerError::RepositoryUnavailable)?;
                report.stale_claims = 1;
            }
            Err(_) => {
                self.dependencies
                    .transactions
                    .rollback(transaction)
                    .await
                    .map_err(|_| PushWorkerError::RepositoryUnavailable)?;
                return Err(PushWorkerError::RepositoryUnavailable);
            }
        }
        Ok(())
    }

    async fn record_failure(
        &self,
        claim: &ClaimedPushDelivery,
        code: PushDeliveryFailureCode,
        max_attempts: u32,
        report: &mut PushWorkerReport,
    ) -> Result<(), PushWorkerError> {
        match self
            .dependencies
            .repository
            .record_delivery_failure(claim, code, self.config.retry_delay, max_attempts)
            .await
            .map_err(|_| PushWorkerError::RepositoryUnavailable)?
        {
            PushDeliveryFailureDisposition::RetryScheduled => report.retries += 1,
            PushDeliveryFailureDisposition::DeadLettered => {
                report.dead_lettered += 1;
                tracing::error!(
                    push_occurrence_id = %claim.claim.occurrence_id,
                    failure_code = code.as_str(),
                    "push delivery reached the terminal dead-letter state"
                );
            }
            PushDeliveryFailureDisposition::StaleClaim => report.stale_claims += 1,
        }
        Ok(())
    }
}

fn normalized_preview(body: &str) -> Option<String> {
    let normalized = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let bounded = normalized.chars().take(80).collect::<String>();
    let bounded = bounded.trim_end();
    (!bounded.is_empty()).then_some(bounded.to_owned())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PushWorkerConfig {
    pub claim_owner: String,
    pub batch_size: u32,
    pub lease_duration: Duration,
    pub provider_timeout: Duration,
    pub lease_safety_margin: Duration,
    pub retry_delay: Duration,
    pub poll_interval: Duration,
    pub max_attempts: u32,
}

impl PushWorkerConfig {
    fn validate(&self) -> Result<(), PushWorkerError> {
        let provider_budget = self
            .provider_timeout
            .checked_add(self.lease_safety_margin)
            .ok_or(PushWorkerError::InvalidConfiguration)?;
        let owner_length = self.claim_owner.chars().count();
        if owner_length == 0
            || owner_length > MAX_PUSH_CLAIM_OWNER_CHARS
            || self.claim_owner.chars().any(char::is_control)
            || self.batch_size == 0
            || self.max_attempts == 0
            || self.provider_timeout.is_zero()
            || self.lease_safety_margin.is_zero()
            || self.lease_duration <= provider_budget
            || self.retry_delay.is_zero()
            || self.poll_interval.is_zero()
        {
            return Err(PushWorkerError::InvalidConfiguration);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PushWorkerReport {
    pub claimed: usize,
    pub succeeded: usize,
    pub retries: usize,
    pub dead_lettered: usize,
    pub invalid_destinations: usize,
    pub authorization_denied: usize,
    pub stale_claims: usize,
}

impl PushWorkerReport {
    fn merge(&mut self, other: Self) {
        self.succeeded += other.succeeded;
        self.retries += other.retries;
        self.dead_lettered += other.dead_lettered;
        self.invalid_destinations += other.invalid_destinations;
        self.authorization_denied += other.authorization_denied;
        self.stale_claims += other.stale_claims;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PushWorkerError {
    InvalidConfiguration,
    RepositoryUnavailable,
}

impl fmt::Display for PushWorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("push worker operation failed")
    }
}

impl std::error::Error for PushWorkerError {}

fn public_installation(record: PushInstallationRecord) -> PushInstallation {
    PushInstallation {
        installation_id: record.installation_id,
        platform: record.platform,
        environment: record.environment,
        provider: record.provider,
        message_preview_enabled: record.message_preview_enabled,
        last_seen_at: record.last_seen_at,
        disabled_at: record.disabled_at,
    }
}

fn validate_bounded(value: &str, maximum: usize) -> Result<(), PushError> {
    let length = value.chars().count();
    if length == 0 || length > maximum || value.chars().any(char::is_control) {
        return Err(PushError::RequestValidation);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpoInstallationCreateInput {
    pub platform: String,
    pub environment: String,
    pub installation_id: String,
    pub expo_token: String,
    pub message_preview_enabled: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpoInstallationPutInput {
    pub expo_token: String,
    pub message_preview_enabled: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PushInstallation {
    pub installation_id: String,
    pub platform: PushPlatform,
    pub environment: PushEnvironment,
    pub provider: PushProviderName,
    pub message_preview_enabled: bool,
    pub last_seen_at: OffsetDateTime,
    pub disabled_at: Option<OffsetDateTime>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PushInstallationUpsert {
    pub installation: PushInstallation,
    pub created: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PushError {
    RequestValidation,
    InstallationNotFound,
    DatabaseUnavailable,
}

impl From<PushRepositoryError> for PushError {
    fn from(error: PushRepositoryError) -> Self {
        match error {
            PushRepositoryError::InstallationNotFound => Self::InstallationNotFound,
            PushRepositoryError::InvalidData | PushRepositoryError::Unavailable => {
                Self::DatabaseUnavailable
            }
        }
    }
}

impl fmt::Display for PushError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("push installation operation failed")
    }
}

impl std::error::Error for PushError {}
