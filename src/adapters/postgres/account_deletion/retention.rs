//! Bounded physical deletion of expired terminal operational rows.

use time::OffsetDateTime;

use super::PostgresAccountDeletionRepository;
use crate::ports::account_deletion::{
    AccountDeletionRepositoryError, AccountDeletionRepositoryFuture, TerminalRetentionReport,
    TerminalRetentionRepository,
};

const DELETES: [(&str, &str, i64); 6] = [
    (
        "DELETE FROM outbox_events WHERE id IN ( \
             SELECT id FROM outbox_events \
             WHERE status = $1 \
               AND published_at < $2::timestamptz - $3::bigint * interval '1 day' \
             ORDER BY published_at, id LIMIT 100 \
         )",
        "published",
        7,
    ),
    (
        "DELETE FROM outbox_events WHERE id IN ( \
             SELECT id FROM outbox_events \
             WHERE status = $1 \
               AND dead_lettered_at < $2::timestamptz - $3::bigint * interval '1 day' \
             ORDER BY dead_lettered_at, id LIMIT 100 \
         )",
        "dead_letter",
        30,
    ),
    (
        "DELETE FROM push_delivery_intents WHERE id IN ( \
             SELECT id FROM push_delivery_intents \
             WHERE status = $1 \
               AND succeeded_at < $2::timestamptz - $3::bigint * interval '1 day' \
             ORDER BY succeeded_at, id LIMIT 100 \
         )",
        "succeeded",
        14,
    ),
    (
        "DELETE FROM push_delivery_intents WHERE id IN ( \
             SELECT id FROM push_delivery_intents \
             WHERE status = $1 \
               AND failed_at < $2::timestamptz - $3::bigint * interval '1 day' \
             ORDER BY failed_at, id LIMIT 100 \
         )",
        "failed",
        30,
    ),
    (
        "DELETE FROM push_delivery_intents WHERE id IN ( \
             SELECT id FROM push_delivery_intents \
             WHERE status = $1 \
               AND dead_lettered_at < $2::timestamptz - $3::bigint * interval '1 day' \
             ORDER BY dead_lettered_at, id LIMIT 100 \
         )",
        "dead_letter",
        30,
    ),
    (
        "DELETE FROM account_object_deletion_intents WHERE id IN ( \
             SELECT id FROM account_object_deletion_intents \
             WHERE status = $1 \
               AND succeeded_at < $2::timestamptz - $3::bigint * interval '1 day' \
             ORDER BY succeeded_at, id LIMIT 100 \
         )",
        "succeeded",
        90,
    ),
];

impl TerminalRetentionRepository for PostgresAccountDeletionRepository {
    fn purge_terminal_rows(&self) -> AccountDeletionRepositoryFuture<'_, TerminalRetentionReport> {
        Box::pin(async move {
            let now = sqlx::query_scalar::<_, OffsetDateTime>("SELECT now()")
                .fetch_one(&self.pool)
                .await
                .map_err(|_| {
                    tracing::warn!(
                        target: "jamye_server",
                        event_kind = "terminal_retention_batch",
                        dependency = "postgres",
                        failure_kind = "retention_clock",
                        "terminal retention database clock query failed"
                    );
                    AccountDeletionRepositoryError::Unavailable
                })?;
            self.purge_terminal_rows_at(now).await
        })
    }
}

impl PostgresAccountDeletionRepository {
    /// Uses a caller-supplied database time to make retention boundaries testable.
    pub async fn purge_terminal_rows_at(
        &self,
        now: OffsetDateTime,
    ) -> Result<TerminalRetentionReport, AccountDeletionRepositoryError> {
        let mut report = TerminalRetentionReport::default();
        let mut failed = false;
        for (index, (query, status, days)) in DELETES.into_iter().enumerate() {
            match sqlx::query(query)
                .bind(status)
                .bind(now)
                .bind(days)
                .execute(&self.pool)
                .await
            {
                Ok(result) => report.deleted[index] = result.rows_affected(),
                Err(error) => {
                    failed = true;
                    let sqlstate = error.as_database_error().and_then(|error| error.code());
                    tracing::warn!(
                        target: "jamye_server",
                        event_kind = "terminal_retention_batch",
                        dependency = "postgres",
                        failure_kind = "retention_delete",
                        sqlstate = sqlstate.as_deref().unwrap_or("unavailable"),
                        "terminal retention delete failed"
                    );
                }
            }
        }
        if failed {
            // Preserve counts from successful statements even when another pair failed.
            if report.deleted.iter().any(|count| *count > 0) {
                report.log_completed();
            }
            Err(AccountDeletionRepositoryError::Unavailable)
        } else {
            Ok(report)
        }
    }
}
