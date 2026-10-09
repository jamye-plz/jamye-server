//! Account purge: blocks are deleted in both directions, reports are retained with their
//! account references moved to the anonymous tombstone, operator alerts of the purged account
//! are removed.

use jamye_server::{
    adapters::postgres::{
        account_deletion::PostgresAccountDeletionRepository, transactions::SqlxTransactionManager,
    },
    ports::{account_deletion::AccountDeletionRepository, transactions::TransactionManager},
};
use uuid::Uuid;

use crate::{
    TestResult,
    support::{
        Harness, insert_group, insert_installation, insert_message, insert_user,
        insert_user_with_id, report_message_body, report_user_body,
    },
};

#[tokio::test]
async fn purge_deletes_blocks_in_both_directions_and_retains_reports_and_snapshots() -> TestResult {
    let purged = Uuid::new_v4();
    // The purged account is also an operator, so alerts reference its installation.
    let harness = Harness::with_operators(vec![purged]).await?;
    let result: TestResult = async {
        insert_user_with_id(&harness.pool, purged, "purged").await?;
        let owner = insert_user(&harness.pool, "owner").await?;
        let other = insert_user(&harness.pool, "other").await?;
        let group = insert_group(&harness.pool, &[owner, purged, other]).await?;
        let other_message =
            insert_message(&harness.pool, group.chatroom_id, other, "from other").await?;
        insert_installation(&harness.pool, purged).await?;

        for (blocker, blocked) in [(purged, other), (other, purged), (owner, other)] {
            sqlx::query("INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1, $2)")
                .bind(blocker)
                .bind(blocked)
                .execute(&harness.pool)
                .await?;
        }
        // Reports: by the purged account, about the purged account, and an unrelated one.
        for (reporter, body) in [
            (purged, report_message_body(other_message, "harassment")),
            (other, report_user_body(purged, "hate")),
            (owner, report_user_body(other, "spam")),
        ] {
            let response = harness.report(reporter, &body).await?;
            assert_eq!(response.status(), axum::http::StatusCode::CREATED);
        }
        let alerts_before = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM push_delivery_intents WHERE recipient_user_id = $1",
        )
        .bind(purged)
        .fetch_one(&harness.pool)
        .await?;
        assert_eq!(alerts_before, 3, "every report alerted the operator installation");

        let repository = PostgresAccountDeletionRepository::new(harness.pool.clone());
        let transactions = SqlxTransactionManager::new(harness.pool.clone());
        let mut transaction = transactions.begin().await?;
        repository
            .start_grace_period(transaction.as_mut(), purged)
            .await?;
        transactions.commit(transaction).await?;
        let mut transaction = transactions.begin().await?;
        repository
            .finalize_deletion(transaction.as_mut(), purged)
            .await?;
        transactions.commit(transaction).await?;

        let blocks = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM user_blocks WHERE blocker_id = $1 OR blocked_id = $1",
        )
        .bind(purged)
        .fetch_one(&harness.pool)
        .await?;
        assert_eq!(blocks, 0, "blocks of the purged account must be deleted both ways");
        let unrelated = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM user_blocks WHERE blocker_id = $1 AND blocked_id = $2",
        )
        .bind(owner)
        .bind(other)
        .fetch_one(&harness.pool)
        .await?;
        assert_eq!(unrelated, 1, "an unrelated block must survive");

        let reports = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM reports")
            .fetch_one(&harness.pool)
            .await?;
        assert_eq!(reports, 3, "reports are retained");
        let still_referenced = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM reports WHERE reporter_id = $1 OR target_user_id = $1",
        )
        .bind(purged)
        .fetch_one(&harness.pool)
        .await?;
        assert_eq!(still_referenced, 0, "reports still reference the purged account");
        let tombstoned = sqlx::query_as::<_, (i64, i64)>(
            "SELECT \
                 count(*) FILTER (WHERE reporter.user_id IS NOT NULL), \
                 count(*) FILTER (WHERE target.user_id IS NOT NULL) \
             FROM reports report \
             LEFT JOIN anonymous_author_tombstones reporter ON reporter.user_id = report.reporter_id \
             LEFT JOIN anonymous_author_tombstones target ON target.user_id = report.target_user_id",
        )
        .fetch_one(&harness.pool)
        .await?;
        assert_eq!(tombstoned, (1, 1));
        let snapshots = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM reports WHERE target_type = 'message' AND message_snapshot IS NOT NULL",
        )
        .fetch_one(&harness.pool)
        .await?;
        assert_eq!(snapshots, 1, "the message snapshot is retained");

        let alerts_after = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM push_delivery_intents WHERE report_id IS NOT NULL",
        )
        .fetch_one(&harness.pool)
        .await?;
        assert_eq!(alerts_after, 0, "alerts of the purged operator are removed");
        let gone = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users WHERE id = $1")
            .bind(purged)
            .fetch_one(&harness.pool)
            .await?;
        assert_eq!(gone, 0);
        Ok(())
    }
    .await;
    harness.finish(result).await
}
