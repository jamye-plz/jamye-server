//! Moderation use cases: reports, user blocks, account suspension, the operator alert and the
//! account gate applied by the bearer extractor.

pub mod admin;

use std::{fmt, sync::Arc, time::Duration};

use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    application::auth::{AccessAccountGate, AccessGateError, AccessGateFuture},
    domain::moderation::{
        ContentFilter, MAX_SUSPENSION_REASON_CHARS, REPORT_RATE_LIMIT, REPORT_RATE_WINDOW_SECONDS,
        ReportReason, bounded_snapshot_text,
    },
    ports::{
        moderation::{
            BlockRecord, BlockUserCommand, BlockedUser, EnqueueOperatorAlertCommand,
            InsertReportCommand, ModerationRepository, ModerationRepositoryError, ReportRecord,
            ReportTarget, ReportedMessage, ResolveReportTargetQuery, SuspendUserCommand,
            UnblockUserCommand,
        },
        rate_limit::{RateLimitOutcome, RateLimitRequest, RateLimiter},
        transactions::{BoxTransactionHandle, TransactionHandle, TransactionManager},
    },
};

const REPORT_RATE_ENDPOINT: &str = "report_create";

#[derive(Clone)]
pub struct ModerationDependencies {
    pub transactions: Arc<dyn TransactionManager>,
    pub repository: Arc<dyn ModerationRepository>,
    pub rate_limiter: Arc<dyn RateLimiter>,
}

#[derive(Clone, Debug, Default)]
pub struct ModerationSettings {
    /// Applied to the report snapshot text (stored message text is already masked).
    pub content_filter: ContentFilter,
    /// Accounts that receive the generic operator alert after a report commits.
    pub operator_account_ids: Vec<Uuid>,
}

#[derive(Clone)]
pub struct ModerationService {
    dependencies: ModerationDependencies,
    settings: ModerationSettings,
}

/// Unvalidated report request fields as received on the wire.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportInput {
    pub target_type: String,
    pub message_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
    pub reason: String,
}

impl ModerationService {
    pub fn new(dependencies: ModerationDependencies, settings: ModerationSettings) -> Self {
        Self {
            dependencies,
            settings,
        }
    }

    /// R1: validate, rate-limit, authorize the target, store the report and snapshot in one
    /// transaction, then enqueue the operator alert after the commit.
    pub async fn report(
        &self,
        reporter_id: Uuid,
        input: ReportInput,
    ) -> Result<ReportRecord, ModerationError> {
        let (target, reason) = parse_report_input(&input)?;
        if target == ReportTarget::User(reporter_id) {
            return Err(ModerationError::ReportSelfTarget);
        }
        self.check_report_rate_limit(reporter_id).await?;

        let mut transaction = self.begin().await?;
        let result = self
            .report_in_transaction(transaction.as_mut(), reporter_id, target, reason)
            .await;
        let record = self.finish(transaction, result).await?;
        self.alert_operators(record).await;
        Ok(record)
    }

    async fn report_in_transaction(
        &self,
        transaction: &mut dyn TransactionHandle,
        reporter_id: Uuid,
        target: ReportTarget,
        reason: ReportReason,
    ) -> Result<ReportRecord, ModerationError> {
        let resolved = self
            .dependencies
            .repository
            .resolve_report_target(
                transaction,
                &ResolveReportTargetQuery {
                    reporter_id,
                    target,
                },
            )
            .await
            .map_err(ModerationError::from)?;
        if let Some(message) = &resolved.message
            && message.author_id == Some(reporter_id)
        {
            return Err(ModerationError::ReportSelfTarget);
        }
        let message_snapshot = resolved
            .message
            .as_ref()
            .map(|message| self.snapshot(message));
        self.dependencies
            .repository
            .insert_report(
                transaction,
                &InsertReportCommand {
                    id: Uuid::new_v4(),
                    reporter_id,
                    target,
                    group_id: resolved.group_id,
                    reason,
                    message_snapshot,
                },
            )
            .await
            .map_err(ModerationError::from)
    }

    /// Message snapshot document: masked text (bounded), content type, media references.
    fn snapshot(&self, message: &ReportedMessage) -> Value {
        let text = message
            .body
            .as_deref()
            .filter(|body| !body.is_empty())
            .map(|body| bounded_snapshot_text(&self.settings.content_filter.mask(body)));
        let content_type = match (text.is_some(), message.media.is_empty()) {
            (true, true) => "text",
            (false, false) => "media",
            (true, false) => "text_and_media",
            (false, true) => "empty",
        };
        let media = message
            .media
            .iter()
            .map(|media| {
                json!({
                    "upload_id": media.upload_id,
                    "kind": media.kind,
                    "object_key": media.object_key,
                })
            })
            .collect::<Vec<_>>();
        json!({
            "text": text,
            "content_type": content_type,
            "media": media,
        })
    }

    /// The alert never fails the report: an enqueue failure is only logged.
    async fn alert_operators(&self, record: ReportRecord) {
        if self.settings.operator_account_ids.is_empty() {
            return;
        }
        let command = EnqueueOperatorAlertCommand {
            report_id: record.id,
            operator_account_ids: self.settings.operator_account_ids.clone(),
        };
        match self
            .dependencies
            .repository
            .enqueue_operator_alerts(&command)
            .await
        {
            Ok(enqueued) => tracing::info!(
                target: "jamye_server",
                event_kind = "operator_report_alert_enqueued",
                report_id = %record.id,
                enqueued,
                "operator report alert enqueued"
            ),
            Err(_) => tracing::warn!(
                target: "jamye_server",
                event_kind = "operator_report_alert_enqueue_failed",
                report_id = %record.id,
                "operator report alert could not be enqueued"
            ),
        }
    }

    /// B1: idempotent block.
    pub async fn block(
        &self,
        blocker_id: Uuid,
        blocked_id: Uuid,
    ) -> Result<BlockRecord, ModerationError> {
        if blocker_id == blocked_id {
            return Err(ModerationError::BlockSelfNotAllowed);
        }
        let mut transaction = self.begin().await?;
        let result = self
            .dependencies
            .repository
            .block_user(
                transaction.as_mut(),
                &BlockUserCommand {
                    blocker_id,
                    blocked_id,
                },
            )
            .await
            .map_err(ModerationError::from);
        self.finish(transaction, result).await
    }

    /// B2: idempotent unblock.
    pub async fn unblock(&self, blocker_id: Uuid, blocked_id: Uuid) -> Result<(), ModerationError> {
        let mut transaction = self.begin().await?;
        let result = self
            .dependencies
            .repository
            .unblock_user(
                transaction.as_mut(),
                &UnblockUserCommand {
                    blocker_id,
                    blocked_id,
                },
            )
            .await
            .map_err(ModerationError::from);
        self.finish(transaction, result).await
    }

    /// B3: every blocked user in one response.
    pub async fn list_blocks(&self, blocker_id: Uuid) -> Result<Vec<BlockedUser>, ModerationError> {
        self.dependencies
            .repository
            .list_blocks(blocker_id)
            .await
            .map_err(ModerationError::from)
    }

    /// Operator action: suspend the account and revoke its refresh sessions.
    pub async fn suspend_user(
        &self,
        user_id: Uuid,
        reason: Option<String>,
    ) -> Result<(), ModerationError> {
        let reason = reason
            .map(|reason| reason.trim().to_owned())
            .filter(|reason| !reason.is_empty());
        if let Some(reason) = &reason
            && (reason.chars().count() > MAX_SUSPENSION_REASON_CHARS
                || reason.chars().any(char::is_control))
        {
            return Err(ModerationError::RequestValidation);
        }
        let mut transaction = self.begin().await?;
        let result = self
            .dependencies
            .repository
            .suspend_user(
                transaction.as_mut(),
                &SuspendUserCommand { user_id, reason },
            )
            .await
            .map_err(ModerationError::from);
        let outcome = self.finish(transaction, result).await;
        if outcome.is_ok() {
            tracing::info!(
                target: "jamye_server",
                event_kind = "account_suspended",
                user_id = %user_id,
                "account suspended"
            );
        }
        outcome
    }

    /// Operator action: clear the suspension. The user signs in again.
    pub async fn unsuspend_user(&self, user_id: Uuid) -> Result<(), ModerationError> {
        let mut transaction = self.begin().await?;
        let result = self
            .dependencies
            .repository
            .unsuspend_user(transaction.as_mut(), user_id)
            .await
            .map_err(ModerationError::from);
        let outcome = self.finish(transaction, result).await;
        if outcome.is_ok() {
            tracing::info!(
                target: "jamye_server",
                event_kind = "account_unsuspended",
                user_id = %user_id,
                "account unsuspended"
            );
        }
        outcome
    }

    async fn check_report_rate_limit(&self, reporter_id: Uuid) -> Result<(), ModerationError> {
        match self
            .dependencies
            .rate_limiter
            .check(&RateLimitRequest {
                endpoint: REPORT_RATE_ENDPOINT,
                subject: format!("user:{reporter_id}"),
                limit: REPORT_RATE_LIMIT,
                window: Duration::from_secs(REPORT_RATE_WINDOW_SECONDS),
            })
            .await
            .map_err(|_| ModerationError::RateLimitUnavailable)?
        {
            RateLimitOutcome::Allowed => Ok(()),
            RateLimitOutcome::Denied { retry_after } => {
                Err(ModerationError::RateLimited { retry_after })
            }
        }
    }

    async fn begin(&self) -> Result<BoxTransactionHandle, ModerationError> {
        self.dependencies
            .transactions
            .begin()
            .await
            .map_err(|_| ModerationError::DatabaseUnavailable)
    }

    async fn finish<T>(
        &self,
        transaction: BoxTransactionHandle,
        result: Result<T, ModerationError>,
    ) -> Result<T, ModerationError> {
        match result {
            Ok(value) => {
                self.dependencies
                    .transactions
                    .commit(transaction)
                    .await
                    .map_err(|_| ModerationError::DatabaseUnavailable)?;
                Ok(value)
            }
            Err(error) => {
                self.dependencies
                    .transactions
                    .rollback(transaction)
                    .await
                    .map_err(|_| ModerationError::DatabaseUnavailable)?;
                Err(error)
            }
        }
    }
}

fn parse_report_input(
    input: &ReportInput,
) -> Result<(ReportTarget, ReportReason), ModerationError> {
    let reason = ReportReason::parse(&input.reason).ok_or(ModerationError::RequestValidation)?;
    let target = match (input.target_type.as_str(), input.message_id, input.user_id) {
        ("message", Some(message_id), None) => ReportTarget::Message(message_id),
        ("user", None, Some(user_id)) => ReportTarget::User(user_id),
        _ => return Err(ModerationError::RequestValidation),
    };
    Ok((target, reason))
}

/// The bearer extractor's account gate: a suspended account gets `403 account_suspended`.
#[derive(Clone)]
pub struct ModerationAccessGate {
    repository: Arc<dyn ModerationRepository>,
}

impl ModerationAccessGate {
    pub fn new(repository: Arc<dyn ModerationRepository>) -> Self {
        Self { repository }
    }
}

impl AccessAccountGate for ModerationAccessGate {
    fn check<'a>(&'a self, user_id: Uuid) -> AccessGateFuture<'a> {
        Box::pin(async move {
            match self.repository.is_suspended(user_id).await {
                Ok(false) => Ok(()),
                Ok(true) => Err(AccessGateError::Suspended),
                Err(_) => Err(AccessGateError::Unavailable),
            }
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModerationError {
    RequestValidation,
    ReportTargetNotFound,
    ReportSelfTarget,
    BlockSelfNotAllowed,
    UserNotFound,
    RateLimited { retry_after: Duration },
    RateLimitUnavailable,
    DatabaseUnavailable,
}

impl fmt::Display for ModerationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("moderation operation failed")
    }
}

impl std::error::Error for ModerationError {}

impl From<ModerationRepositoryError> for ModerationError {
    fn from(error: ModerationRepositoryError) -> Self {
        match error {
            ModerationRepositoryError::TargetNotFound => Self::ReportTargetNotFound,
            ModerationRepositoryError::UserNotFound => Self::UserNotFound,
            ModerationRepositoryError::InvalidData | ModerationRepositoryError::Unavailable => {
                Self::DatabaseUnavailable
            }
        }
    }
}
