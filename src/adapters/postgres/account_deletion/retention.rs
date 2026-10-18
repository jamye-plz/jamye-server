//! Bounded physical deletion of expired terminal operational rows.

use time::OffsetDateTime;

use super::PostgresAccountDeletionRepository;
use crate::ports::account_deletion::{
    AccountDeletionRepositoryError, AccountDeletionRepositoryFuture, TerminalRetentionReport,
    TerminalRetentionRepository,
};

const DELETES: [(&str, &str); 6] = [
    (
        "DELETE FROM outbox_events WHERE id IN (SELECT id FROM outbox_events WHERE status = $1 AND published_at < $2 ORDER BY published_at, id LIMIT 100)",
        "published",
    ),
    (
        "DELETE FROM outbox_events WHERE id IN (SELECT id FROM outbox_events WHERE status = $1 AND dead_lettered_at < $2 ORDER BY dead_lettered_at, id LIMIT 100)",
        "dead_letter",
    ),
    (
        "DELETE FROM push_delivery_intents WHERE id IN (SELECT id FROM push_delivery_intents WHERE status = $1 AND succeeded_at < $2 ORDER BY succeeded_at, id LIMIT 100)",
        "succeeded",
    ),
    (
        "DELETE FROM push_delivery_intents WHERE id IN (SELECT id FROM push_delivery_intents WHERE status = $1 AND failed_at < $2 ORDER BY failed_at, id LIMIT 100)",
        "failed",
    ),
    (
        "DELETE FROM push_delivery_intents WHERE id IN (SELECT id FROM push_delivery_intents WHERE status = $1 AND dead_lettered_at < $2 ORDER BY dead_lettered_at, id LIMIT 100)",
        "dead_letter",
    ),
    (
        "DELETE FROM account_object_deletion_intents WHERE id IN (SELECT id FROM account_object_deletion_intents WHERE status = $1 AND succeeded_at < $2 ORDER BY succeeded_at, id LIMIT 100)",
        "succeeded",
    ),
];

impl TerminalRetentionRepository for PostgresAccountDeletionRepository {
    fn purge_terminal_rows(&self) -> AccountDeletionRepositoryFuture<'_, TerminalRetentionReport> {
        Box::pin(async move {
            let now = sqlx::query_scalar::<_, OffsetDateTime>("SELECT now()")
                .fetch_one(&self.pool)
                .await
                .map_err(|_| AccountDeletionRepositoryError::Unavailable)?;
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
        let (days_7, days_14, days_30, days_90): (
            OffsetDateTime,
            OffsetDateTime,
            OffsetDateTime,
            OffsetDateTime,
        ) = sqlx::query_as("SELECT $1::timestamptz - interval '7 days', $1::timestamptz - interval '14 days', $1::timestamptz - interval '30 days', $1::timestamptz - interval '90 days'")
            .bind(now)
            .fetch_one(&self.pool)
            .await
            .map_err(|_| AccountDeletionRepositoryError::Unavailable)?;
        let cutoffs = [days_7, days_30, days_14, days_30, days_30, days_90];
        let mut report = TerminalRetentionReport::default();
        let mut failed = false;
        for (index, ((query, status), cutoff)) in DELETES.into_iter().zip(cutoffs).enumerate() {
            match sqlx::query(query)
                .bind(status)
                .bind(cutoff)
                .execute(&self.pool)
                .await
            {
                Ok(result) => {
                    report.deleted[index] = result.rows_affected();
                    tracing::info!(
                        pair = index,
                        deleted = report.deleted[index],
                        "terminal retention delete completed"
                    );
                }
                Err(_) => {
                    failed = true;
                    tracing::warn!(pair = index, "terminal retention delete failed");
                }
            }
        }
        if failed {
            Err(AccountDeletionRepositoryError::Unavailable)
        } else {
            Ok(report)
        }
    }
}
