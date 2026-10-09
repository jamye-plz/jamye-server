//! Authoritative PostgreSQL moderation persistence boundary: reports, user blocks,
//! account suspension and the operator report alert.

use std::{fmt, future::Future, pin::Pin};

use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    domain::moderation::{ReportReason, ReportStatus},
    ports::transactions::TransactionHandle,
};

pub type ModerationRepositoryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ModerationRepositoryError>> + Send + 'a>>;

pub trait ModerationRepository: Send + Sync {
    /// Idempotent one-directional block; returns the original `blocked_at`.
    fn block_user<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a BlockUserCommand,
    ) -> ModerationRepositoryFuture<'a, BlockRecord>;

    /// Idempotent; also succeeds when no block exists.
    fn unblock_user<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a UnblockUserCommand,
    ) -> ModerationRepositoryFuture<'a, ()>;

    /// Every blocked user, `blocked_at` descending then `user_id`.
    fn list_blocks(&self, blocker_id: Uuid) -> ModerationRepositoryFuture<'_, Vec<BlockedUser>>;

    /// Authorize the reporter against the target and read the data a snapshot needs. A missing
    /// and an inaccessible target are the same `TargetNotFound`.
    fn resolve_report_target<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        query: &'a ResolveReportTargetQuery,
    ) -> ModerationRepositoryFuture<'a, ResolvedReportTarget>;

    fn insert_report<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a InsertReportCommand,
    ) -> ModerationRepositoryFuture<'a, ReportRecord>;

    /// Enqueue one content-free alert occurrence per active installation of each operator
    /// account through the push delivery table. Runs after the report commit in its own
    /// statement; returns the number of occurrences created.
    fn enqueue_operator_alerts<'a>(
        &'a self,
        command: &'a EnqueueOperatorAlertCommand,
    ) -> ModerationRepositoryFuture<'a, u64>;

    /// `true` when the live account is suspended.
    fn is_suspended(&self, user_id: Uuid) -> ModerationRepositoryFuture<'_, bool>;

    /// Suspend (idempotent) and revoke every refresh session of the account.
    fn suspend_user<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a SuspendUserCommand,
    ) -> ModerationRepositoryFuture<'a, ()>;

    /// Clear the suspension (idempotent). Revoked sessions stay revoked: the user signs in again.
    fn unsuspend_user<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        user_id: Uuid,
    ) -> ModerationRepositoryFuture<'a, ()>;
}

/// Operator-facing report queue and handling (admin CLI). Reads never return emails or tokens:
/// reports carry only ids, the reason, timestamps and the stored snapshot.
pub trait ModerationAdminRepository: Send + Sync {
    /// Reports with `status` (every status when `None`), oldest first, at most `limit`.
    fn list_reports(
        &self,
        status: Option<ReportStatus>,
        limit: u32,
    ) -> ModerationRepositoryFuture<'_, Vec<ReportDetail>>;

    fn find_report(&self, report_id: Uuid) -> ModerationRepositoryFuture<'_, Option<ReportDetail>>;

    /// Move an open report to `Actioned` or `Dismissed`. Repeating the same transition is
    /// `Unchanged`; the other terminal status is a `Conflict`.
    fn handle_report<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        report_id: Uuid,
        status: ReportStatus,
    ) -> ModerationRepositoryFuture<'a, ReportHandling>;

    /// Mark every open report that targets `message_id` as actioned; returns their ids.
    fn action_message_reports<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        message_id: Uuid,
    ) -> ModerationRepositoryFuture<'a, Vec<Uuid>>;

    /// Retention: delete handled (actioned or dismissed) reports whose `handled_at` is more than
    /// `older_than_days` days old, together with their operator-alert push occurrences. Open
    /// reports are never deleted. Returns the number of reports deleted.
    fn purge_handled_reports<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        older_than_days: u32,
    ) -> ModerationRepositoryFuture<'a, u64>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReportDetail {
    pub id: Uuid,
    pub status: ReportStatus,
    pub reason: ReportReason,
    pub created_at: OffsetDateTime,
    pub handled_at: Option<OffsetDateTime>,
    pub reporter_id: Uuid,
    pub group_id: Uuid,
    pub target: ReportTarget,
    /// The stored message snapshot document (`text`, `content_type`, `media`); `None` for a
    /// user target.
    pub snapshot: Option<serde_json::Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportHandling {
    Updated,
    /// The report already has the requested status.
    Unchanged,
    /// The report was already handled with the other terminal status.
    Conflict {
        current: ReportStatus,
    },
    NotFound,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockUserCommand {
    pub blocker_id: Uuid,
    pub blocked_id: Uuid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnblockUserCommand {
    pub blocker_id: Uuid,
    pub blocked_id: Uuid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockRecord {
    pub blocked_id: Uuid,
    pub blocked_at: OffsetDateTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockedUser {
    pub user_id: Uuid,
    pub nickname: String,
    pub avatar_url: Option<String>,
    pub blocked_at: OffsetDateTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportTarget {
    Message(Uuid),
    User(Uuid),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolveReportTargetQuery {
    pub reporter_id: Uuid,
    pub target: ReportTarget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedReportTarget {
    /// The group the report is filed under (the message's group, or a group shared with the user).
    pub group_id: Uuid,
    /// `Some` for a message target.
    pub message: Option<ReportedMessage>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportedMessage {
    pub author_id: Option<Uuid>,
    pub body: Option<String>,
    pub media: Vec<ReportedMedia>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportedMedia {
    pub upload_id: Uuid,
    pub kind: String,
    pub object_key: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InsertReportCommand {
    pub id: Uuid,
    pub reporter_id: Uuid,
    pub target: ReportTarget,
    pub group_id: Uuid,
    pub reason: ReportReason,
    /// `Some` for a message target: the bounded, masked snapshot document.
    pub message_snapshot: Option<serde_json::Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReportRecord {
    pub id: Uuid,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnqueueOperatorAlertCommand {
    pub report_id: Uuid,
    pub operator_account_ids: Vec<Uuid>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SuspendUserCommand {
    pub user_id: Uuid,
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModerationRepositoryError {
    /// The report target is missing or not accessible to the reporter.
    TargetNotFound,
    /// The block or suspension subject does not exist (or is a deleted account).
    UserNotFound,
    InvalidData,
    Unavailable,
}

impl fmt::Display for ModerationRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("moderation persistence operation failed")
    }
}

impl std::error::Error for ModerationRepositoryError {}
