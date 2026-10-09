//! Operator use cases behind the admin CLI: the report queue, moderator message deletion and the
//! report retention purge. Every mutation is one transaction and emits one structured log line
//! (there is no audit table).

use std::{fmt, sync::Arc};

use uuid::Uuid;

use crate::{
    domain::moderation::ReportStatus,
    ports::{
        messaging::{
            MessagingRepository, MessagingRepositoryError, ModeratorDeleteMessageCommand,
            ModeratorDeleteOutcome,
        },
        moderation::{
            ModerationAdminRepository, ModerationRepositoryError, ReportDetail, ReportHandling,
        },
        transactions::{BoxTransactionHandle, TransactionHandle, TransactionManager},
    },
};

/// Most reports one `reports list` returns.
pub const MAX_REPORT_LIST_LIMIT: u32 = 200;
/// Longest accepted retention for `reports purge` (ten years); zero is rejected so a typo can
/// never delete every handled report.
pub const MAX_PURGE_RETENTION_DAYS: u32 = 3650;

#[derive(Clone)]
pub struct ModerationAdminDependencies {
    pub transactions: Arc<dyn TransactionManager>,
    pub repository: Arc<dyn ModerationAdminRepository>,
    pub messaging: Arc<dyn MessagingRepository>,
}

#[derive(Clone)]
pub struct ModerationAdminService {
    dependencies: ModerationAdminDependencies,
}

/// Result of `messages delete`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageDeletion {
    pub message_id: Uuid,
    pub chatroom_id: Uuid,
    pub group_id: Uuid,
    /// The message was already deleted (author or earlier operator delete): nothing changed
    /// except that any still-open report on it was actioned.
    pub already_deleted: bool,
    /// Open reports on the message that were marked actioned, sorted.
    pub actioned_reports: Vec<Uuid>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportChange {
    Changed,
    /// The report already had the requested status.
    Unchanged,
}

impl ModerationAdminService {
    pub fn new(dependencies: ModerationAdminDependencies) -> Self {
        Self { dependencies }
    }

    /// Reports with `status` (all statuses when `None`), oldest first.
    pub async fn list_reports(
        &self,
        status: Option<ReportStatus>,
        limit: u32,
    ) -> Result<Vec<ReportDetail>, ModerationAdminError> {
        if limit == 0 || limit > MAX_REPORT_LIST_LIMIT {
            return Err(ModerationAdminError::InvalidInput);
        }
        self.dependencies
            .repository
            .list_reports(status, limit)
            .await
            .map_err(ModerationAdminError::from)
    }

    pub async fn show_report(&self, report_id: Uuid) -> Result<ReportDetail, ModerationAdminError> {
        self.dependencies
            .repository
            .find_report(report_id)
            .await
            .map_err(ModerationAdminError::from)?
            .ok_or(ModerationAdminError::ReportNotFound)
    }

    /// Close a report without action.
    pub async fn dismiss_report(
        &self,
        report_id: Uuid,
    ) -> Result<ReportChange, ModerationAdminError> {
        self.handle_report(report_id, ReportStatus::Dismissed).await
    }

    /// Close a report after the operator acted on it.
    pub async fn resolve_report(
        &self,
        report_id: Uuid,
    ) -> Result<ReportChange, ModerationAdminError> {
        self.handle_report(report_id, ReportStatus::Actioned).await
    }

    async fn handle_report(
        &self,
        report_id: Uuid,
        status: ReportStatus,
    ) -> Result<ReportChange, ModerationAdminError> {
        let mut transaction = self.begin().await?;
        let result = self
            .dependencies
            .repository
            .handle_report(transaction.as_mut(), report_id, status)
            .await
            .map_err(ModerationAdminError::from);
        match self.finish(transaction, result).await? {
            ReportHandling::Updated => {
                tracing::info!(
                    target: "jamye_server",
                    event_kind = "admin_report_handled",
                    report_id = %report_id,
                    status = status.as_str(),
                    "operator handled a report"
                );
                Ok(ReportChange::Changed)
            }
            ReportHandling::Unchanged => Ok(ReportChange::Unchanged),
            ReportHandling::Conflict { current } => {
                Err(ModerationAdminError::ReportConflict { current })
            }
            ReportHandling::NotFound => Err(ModerationAdminError::ReportNotFound),
        }
    }

    /// Moderator deletion: the author-delete effects (soft delete, tombstone, delete event for
    /// sync, media cleanup queue) plus actioning every open report on the message, in one
    /// transaction. Idempotent.
    pub async fn delete_message(
        &self,
        message_id: Uuid,
    ) -> Result<MessageDeletion, ModerationAdminError> {
        let mut transaction = self.begin().await?;
        let result = self
            .delete_message_in_transaction(transaction.as_mut(), message_id)
            .await;
        let deletion = self.finish(transaction, result).await?;
        tracing::info!(
            target: "jamye_server",
            event_kind = "admin_message_deleted",
            message_id = %deletion.message_id,
            chatroom_id = %deletion.chatroom_id,
            group_id = %deletion.group_id,
            already_deleted = deletion.already_deleted,
            actioned_reports = deletion.actioned_reports.len(),
            "operator deleted a message"
        );
        Ok(deletion)
    }

    async fn delete_message_in_transaction(
        &self,
        transaction: &mut dyn TransactionHandle,
        message_id: Uuid,
    ) -> Result<MessageDeletion, ModerationAdminError> {
        let outcome = self
            .dependencies
            .messaging
            .moderator_delete_message(transaction, &ModeratorDeleteMessageCommand { message_id })
            .await
            .map_err(|error| match error {
                MessagingRepositoryError::MessageNotFound => ModerationAdminError::MessageNotFound,
                _ => ModerationAdminError::DatabaseUnavailable,
            })?;
        let (moderated, already_deleted) = match outcome {
            ModeratorDeleteOutcome::Deleted(moderated) => (moderated, false),
            ModeratorDeleteOutcome::AlreadyDeleted(moderated) => (moderated, true),
            ModeratorDeleteOutcome::NotUserMessage => {
                return Err(ModerationAdminError::NotUserMessage);
            }
        };
        let mut actioned_reports = self
            .dependencies
            .repository
            .action_message_reports(transaction, message_id)
            .await
            .map_err(ModerationAdminError::from)?;
        actioned_reports.sort();
        Ok(MessageDeletion {
            message_id,
            chatroom_id: moderated.chatroom_id,
            group_id: moderated.group_id,
            already_deleted,
            actioned_reports,
        })
    }

    /// Retention purge of handled reports older than `older_than_days`; returns the count.
    pub async fn purge_handled_reports(
        &self,
        older_than_days: u32,
    ) -> Result<u64, ModerationAdminError> {
        if older_than_days == 0 || older_than_days > MAX_PURGE_RETENTION_DAYS {
            return Err(ModerationAdminError::InvalidInput);
        }
        let mut transaction = self.begin().await?;
        let result = self
            .dependencies
            .repository
            .purge_handled_reports(transaction.as_mut(), older_than_days)
            .await
            .map_err(ModerationAdminError::from);
        let deleted = self.finish(transaction, result).await?;
        tracing::info!(
            target: "jamye_server",
            event_kind = "admin_reports_purged",
            older_than_days,
            deleted,
            "operator purged handled reports"
        );
        Ok(deleted)
    }

    async fn begin(&self) -> Result<BoxTransactionHandle, ModerationAdminError> {
        self.dependencies
            .transactions
            .begin()
            .await
            .map_err(|_| ModerationAdminError::DatabaseUnavailable)
    }

    async fn finish<T>(
        &self,
        transaction: BoxTransactionHandle,
        result: Result<T, ModerationAdminError>,
    ) -> Result<T, ModerationAdminError> {
        match result {
            Ok(value) => {
                self.dependencies
                    .transactions
                    .commit(transaction)
                    .await
                    .map_err(|_| ModerationAdminError::DatabaseUnavailable)?;
                Ok(value)
            }
            Err(error) => {
                self.dependencies
                    .transactions
                    .rollback(transaction)
                    .await
                    .map_err(|_| ModerationAdminError::DatabaseUnavailable)?;
                Err(error)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModerationAdminError {
    InvalidInput,
    ReportNotFound,
    /// The report was already handled with the other terminal status.
    ReportConflict {
        current: ReportStatus,
    },
    /// The message does not exist, or its group, topic or chatroom no longer does.
    MessageNotFound,
    NotUserMessage,
    DatabaseUnavailable,
}

impl fmt::Display for ModerationAdminError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("moderation admin operation failed")
    }
}

impl std::error::Error for ModerationAdminError {}

impl From<ModerationRepositoryError> for ModerationAdminError {
    fn from(error: ModerationRepositoryError) -> Self {
        match error {
            ModerationRepositoryError::TargetNotFound
            | ModerationRepositoryError::UserNotFound
            | ModerationRepositoryError::InvalidData
            | ModerationRepositoryError::Unavailable => Self::DatabaseUnavailable,
        }
    }
}
