//! Operator report queue for the admin CLI: list, show, handle, and action the open reports of a
//! moderated message. Every statement is parameterized; no row exposes emails or tokens.

use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{PostgresModerationRepository, database_error};
use crate::{
    adapters::postgres::transactions::connection,
    domain::moderation::{ReportReason, ReportStatus},
    ports::{
        moderation::{
            ModerationAdminRepository, ModerationRepositoryError, ModerationRepositoryFuture,
            ReportDetail, ReportHandling, ReportTarget,
        },
        transactions::TransactionHandle,
    },
};

type ReportRow = (
    Uuid,
    String,
    String,
    OffsetDateTime,
    Option<OffsetDateTime>,
    Uuid,
    Uuid,
    String,
    Option<Uuid>,
    Option<Uuid>,
    Option<Value>,
);

impl ModerationAdminRepository for PostgresModerationRepository {
    fn list_reports(
        &self,
        status: Option<ReportStatus>,
        limit: u32,
    ) -> ModerationRepositoryFuture<'_, Vec<ReportDetail>> {
        Box::pin(async move {
            let rows = sqlx::query_as::<_, ReportRow>(
                "SELECT id, status, reason, created_at, handled_at, reporter_id, \
                        target_group_id, target_type, target_message_id, target_user_id, \
                        message_snapshot \
                 FROM reports \
                 WHERE ($1::TEXT IS NULL OR status = $1) \
                 ORDER BY created_at, id \
                 LIMIT $2",
            )
            .bind(status.map(ReportStatus::as_str))
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(|error| database_error("moderation_report_list", error))?;
            rows.into_iter()
                .map(report_detail)
                .collect::<Result<Vec<_>, _>>()
        })
    }

    fn find_report(&self, report_id: Uuid) -> ModerationRepositoryFuture<'_, Option<ReportDetail>> {
        Box::pin(async move {
            sqlx::query_as::<_, ReportRow>(
                "SELECT id, status, reason, created_at, handled_at, reporter_id, \
                        target_group_id, target_type, target_message_id, target_user_id, \
                        message_snapshot \
                 FROM reports \
                 WHERE id = $1",
            )
            .bind(report_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| database_error("moderation_report_find", error))?
            .map(report_detail)
            .transpose()
        })
    }

    fn handle_report<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        report_id: Uuid,
        status: ReportStatus,
    ) -> ModerationRepositoryFuture<'a, ReportHandling> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| ModerationRepositoryError::InvalidData)?;
            let current = sqlx::query_scalar::<_, String>(
                "SELECT status FROM reports WHERE id = $1 FOR UPDATE",
            )
            .bind(report_id)
            .fetch_optional(&mut *connection)
            .await
            .map_err(|error| database_error("moderation_report_lock", error))?;
            let Some(current) = current else {
                return Ok(ReportHandling::NotFound);
            };
            let current =
                ReportStatus::parse(&current).ok_or(ModerationRepositoryError::InvalidData)?;
            if current == status {
                return Ok(ReportHandling::Unchanged);
            }
            if current != ReportStatus::Open {
                return Ok(ReportHandling::Conflict { current });
            }
            sqlx::query(
                "UPDATE reports SET status = $2, handled_at = clock_timestamp() WHERE id = $1",
            )
            .bind(report_id)
            .bind(status.as_str())
            .execute(connection)
            .await
            .map_err(|error| database_error("moderation_report_handle", error))?;
            Ok(ReportHandling::Updated)
        })
    }

    fn action_message_reports<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        message_id: Uuid,
    ) -> ModerationRepositoryFuture<'a, Vec<Uuid>> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| ModerationRepositoryError::InvalidData)?;
            sqlx::query_scalar::<_, Uuid>(
                "UPDATE reports \
                 SET status = 'actioned', handled_at = clock_timestamp() \
                 WHERE target_message_id = $1 AND status = 'open' \
                 RETURNING id",
            )
            .bind(message_id)
            .fetch_all(connection)
            .await
            .map_err(|error| database_error("moderation_report_action_message", error))
        })
    }

    fn purge_handled_reports<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        older_than_days: u32,
    ) -> ModerationRepositoryFuture<'a, u64> {
        Box::pin(async move {
            let days = i32::try_from(older_than_days)
                .map_err(|_| ModerationRepositoryError::InvalidData)?;
            let connection =
                connection(transaction).map_err(|_| ModerationRepositoryError::InvalidData)?;
            // One cutoff for both statements, so a report cannot expire between them.
            let cutoff = sqlx::query_scalar::<_, OffsetDateTime>(
                "SELECT clock_timestamp() - make_interval(days => $1)",
            )
            .bind(days)
            .fetch_one(&mut *connection)
            .await
            .map_err(|error| database_error("moderation_report_purge_cutoff", error))?;
            // push_delivery_intents.report_id has no ON DELETE action: alerts go first.
            sqlx::query(
                "DELETE FROM push_delivery_intents \
                 WHERE report_id IN ( \
                     SELECT id FROM reports \
                     WHERE status <> 'open' AND handled_at < $1)",
            )
            .bind(cutoff)
            .execute(&mut *connection)
            .await
            .map_err(|error| database_error("moderation_report_purge_alerts", error))?;
            sqlx::query("DELETE FROM reports WHERE status <> 'open' AND handled_at < $1")
                .bind(cutoff)
                .execute(connection)
                .await
                .map(|result| result.rows_affected())
                .map_err(|error| database_error("moderation_report_purge", error))
        })
    }
}

fn report_detail(row: ReportRow) -> Result<ReportDetail, ModerationRepositoryError> {
    let (
        id,
        status,
        reason,
        created_at,
        handled_at,
        reporter_id,
        group_id,
        target_type,
        target_message_id,
        target_user_id,
        snapshot,
    ) = row;
    let target = match (target_type.as_str(), target_message_id, target_user_id) {
        ("message", Some(message_id), None) => ReportTarget::Message(message_id),
        ("user", None, Some(user_id)) => ReportTarget::User(user_id),
        _ => return Err(ModerationRepositoryError::InvalidData),
    };
    Ok(ReportDetail {
        id,
        status: ReportStatus::parse(&status).ok_or(ModerationRepositoryError::InvalidData)?,
        reason: ReportReason::parse(&reason).ok_or(ModerationRepositoryError::InvalidData)?,
        created_at,
        handled_at,
        reporter_id,
        group_id,
        target,
        snapshot,
    })
}
