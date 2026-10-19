use std::sync::Arc;

use futures_util::stream;
use jamye_server::{
    adapters::postgres::account_deletion::PostgresAccountDeletionRepository,
    application::account_deletion::cleanup::run_terminal_retention_until,
    platform::logging::build_json_subscriber,
    ports::account_deletion::{
        AccountDeletionRepositoryError, AccountDeletionRepositoryFuture, TerminalRetentionReport,
        TerminalRetentionRepository,
    },
};
use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;
use tokio::sync::{mpsc, oneshot};
use tracing::instrument::WithSubscriber;
use uuid::Uuid;

use crate::{
    TestResult, logging_support::SharedWriter, postgres_support::TestDatabase,
    support::finish_database_test,
};

#[derive(Clone, Copy)]
enum Table {
    Outbox,
    Push,
    Object,
}

const PAIRS: [(Table, &str, i64); 6] = [
    (Table::Outbox, "published", 7),
    (Table::Outbox, "dead_letter", 30),
    (Table::Push, "succeeded", 14),
    (Table::Push, "failed", 30),
    (Table::Push, "dead_letter", 30),
    (Table::Object, "succeeded", 90),
];

struct Seeds {
    user: Uuid,
    notification: Uuid,
    installation: Uuid,
}

impl Seeds {
    async fn new(pool: &PgPool) -> TestResult<Self> {
        let seeds = Self {
            user: Uuid::new_v4(),
            notification: Uuid::new_v4(),
            installation: Uuid::new_v4(),
        };
        sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, 'retention')")
            .bind(seeds.user)
            .execute(pool)
            .await?;
        sqlx::query(
            "INSERT INTO notifications (id, user_id, type, payload) VALUES ($1, $2, 'other', '{}')",
        )
        .bind(seeds.notification)
        .bind(seeds.user)
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO push_installations \
                 (id, user_id, installation_id, platform, token, environment) \
             VALUES ($1, $2, 'retention', 'ios', 'retention-token', 'development')",
        )
        .bind(seeds.installation)
        .bind(seeds.user)
        .execute(pool)
        .await?;
        Ok(seeds)
    }

    async fn event(&self, pool: &PgPool) -> TestResult<Uuid> {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO conversation_events \
                 (id, conversation_id, event_type, event_version, payload, occurred_at) \
             VALUES ($1, $2, 'retention', 1, '{}', now() - interval '1000 days')",
        )
        .bind(id)
        .bind(Uuid::new_v4())
        .execute(pool)
        .await?;
        Ok(id)
    }

    async fn insert(
        &self,
        pool: &PgPool,
        table: Table,
        status: &str,
        at: OffsetDateTime,
    ) -> TestResult<Uuid> {
        let id = Uuid::new_v4();
        match table {
            Table::Outbox => {
                sqlx::query(
                    "INSERT INTO outbox_events \
                         (id, intent_type, event_type, event_version, aggregate_type, \
                          aggregate_id, payload, status, published_at, dead_lettered_at, \
                          created_at, claim_owner, claim_expires_at) \
                     VALUES ($1, 'control', 'retention', 1, 'group', $2, '{}', $3, \
                             CASE WHEN $3 = 'published' THEN $4::timestamptz END, \
                             CASE WHEN $3 = 'dead_letter' THEN $4::timestamptz END, \
                             $4::timestamptz - interval '1 day', \
                             CASE WHEN $3 = 'claimed' THEN 'owner' END, \
                             CASE WHEN $3 = 'claimed' THEN $4::timestamptz END)",
                )
                .bind(id)
                .bind(Uuid::new_v4())
                .bind(status)
                .bind(at)
                .execute(pool)
                .await?;
            }
            Table::Push => {
                let event = self.event(pool).await?;
                sqlx::query(
                    "INSERT INTO push_delivery_intents \
                         (id, notification_id, source_event_id, recipient_user_id, \
                          push_installation_id, installation_owner_epoch, \
                          message_preview_enabled_snapshot, payload, status, succeeded_at, \
                          failed_at, dead_lettered_at, created_at, claim_owner, lease_expires_at) \
                     VALUES ($1, $2, $3, $4, $5, 1, false, '{}', $6, \
                             CASE WHEN $6 = 'succeeded' THEN $7::timestamptz END, \
                             CASE WHEN $6 = 'failed' THEN $7::timestamptz END, \
                             CASE WHEN $6 = 'dead_letter' THEN $7::timestamptz END, \
                             $7::timestamptz - interval '1 day', \
                             CASE WHEN $6 = 'claimed' THEN 'owner' END, \
                             CASE WHEN $6 = 'claimed' THEN $7::timestamptz END)",
                )
                .bind(id)
                .bind(self.notification)
                .bind(event)
                .bind(self.user)
                .bind(self.installation)
                .bind(status)
                .bind(at)
                .execute(pool)
                .await?;
            }
            Table::Object => {
                sqlx::query(
                    "INSERT INTO account_object_deletion_intents \
                         (id, object_key, status, succeeded_at, failed_at, dead_lettered_at, \
                          created_at, claim_owner, lease_expires_at) \
                     VALUES ($1, $2, $3, \
                             CASE WHEN $3 = 'succeeded' THEN $4::timestamptz END, \
                             CASE WHEN $3 = 'failed' THEN $4::timestamptz END, \
                             CASE WHEN $3 = 'dead_letter' THEN $4::timestamptz END, \
                             $4::timestamptz - interval '1 day', \
                             CASE WHEN $3 = 'claimed' THEN 'owner' END, \
                             CASE WHEN $3 = 'claimed' THEN $4::timestamptz END)",
                )
                .bind(id)
                .bind(format!("retention/{id}"))
                .bind(status)
                .bind(at)
                .execute(pool)
                .await?;
            }
        }
        Ok(id)
    }
}

async fn exists(pool: &PgPool, table: Table, id: Uuid) -> TestResult<bool> {
    let query = match table {
        Table::Outbox => "SELECT EXISTS (SELECT 1 FROM outbox_events WHERE id = $1)",
        Table::Push => "SELECT EXISTS (SELECT 1 FROM push_delivery_intents WHERE id = $1)",
        Table::Object => {
            "SELECT EXISTS (SELECT 1 FROM account_object_deletion_intents WHERE id = $1)"
        }
    };
    Ok(sqlx::query_scalar(query).bind(id).fetch_one(pool).await?)
}

const COMPLETION_FIELDS: [&str; 6] = [
    "outbox_events_published",
    "outbox_events_dead_letter",
    "push_delivery_intents_succeeded",
    "push_delivery_intents_failed",
    "push_delivery_intents_dead_letter",
    "account_object_deletion_intents_succeeded",
];

fn entries_with_message<'a>(entries: &'a [Value], message: &str) -> Vec<&'a Value> {
    entries
        .iter()
        .filter(|entry| entry.get("message").and_then(Value::as_str) == Some(message))
        .collect()
}

fn assert_completion_event(entry: &Value, report: TerminalRetentionReport) {
    assert_eq!(entry["target"], "jamye_server");
    assert_eq!(entry["event_kind"], "terminal_retention_batch");
    assert_eq!(entry["message"], "terminal retention batch completed");
    for (field, expected) in COMPLETION_FIELDS.into_iter().zip(report.deleted) {
        assert_eq!(entry[field].as_u64(), Some(expected), "wrong {field}");
    }
}

#[tokio::test]
async fn terminal_retention_boundaries_by_table_and_state() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let now: OffsetDateTime = sqlx::query_scalar("SELECT now()").fetch_one(&pool).await?;
        let seeds = Seeds::new(&pool).await?;
        let mut cases = Vec::new();
        for (table, status, days) in PAIRS {
            let cutoff: OffsetDateTime =
                sqlx::query_scalar("SELECT $1::timestamptz - $2::bigint * interval '1 day'")
                    .bind(now)
                    .bind(days)
                    .fetch_one(&pool)
                    .await?;
            for (offset, retained) in [(-1, false), (0, true), (1, true)] {
                let id = seeds
                    .insert(
                        &pool,
                        table,
                        status,
                        cutoff + time::Duration::microseconds(offset),
                    )
                    .await?;
                if offset == 0 {
                    let query = match table {
                        Table::Outbox => "UPDATE outbox_events SET deleted_at = $2 WHERE id = $1",
                        Table::Push => {
                            "UPDATE push_delivery_intents SET deleted_at = $2 WHERE id = $1"
                        }
                        Table::Object => {
                            "UPDATE account_object_deletion_intents \
                             SET deleted_at = $2 WHERE id = $1"
                        }
                    };
                    sqlx::query(query)
                        .bind(id)
                        .bind(now - time::Duration::days(1000))
                        .execute(&pool)
                        .await?;
                }
                cases.push((table, id, retained));
            }
        }
        for (table, status) in [
            (Table::Outbox, "pending"),
            (Table::Outbox, "claimed"),
            (Table::Push, "pending"),
            (Table::Push, "claimed"),
            (Table::Push, "retryable"),
            (Table::Object, "pending"),
            (Table::Object, "claimed"),
            (Table::Object, "retryable"),
            (Table::Object, "failed"),
            (Table::Object, "dead_letter"),
        ] {
            let id = seeds
                .insert(&pool, table, status, now - time::Duration::days(1000))
                .await?;
            cases.push((table, id, true));
        }
        let event = seeds.event(&pool).await?;
        let repo = PostgresAccountDeletionRepository::new(pool.clone());
        let report = repo.purge_terminal_rows_at(now).await?;
        assert_eq!(report.deleted, [1; 6]);
        for (table, id, retained) in cases {
            assert_eq!(
                exists(&pool, table, id).await?,
                retained,
                "unexpected retention for {id}"
            );
        }
        let event_exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM conversation_events WHERE id = $1)")
                .bind(event)
                .fetch_one(&pool)
                .await?;
        assert!(event_exists);
        Ok(())
    }
    .await;
    finish_database_test(database, pool, result).await
}

#[tokio::test]
async fn bounded_batches_and_repeat_are_safe() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let now: OffsetDateTime = sqlx::query_scalar("SELECT now()").fetch_one(&pool).await?;
        let seeds = Seeds::new(&pool).await?;
        let mut batches = Vec::new();
        for (table, status, days) in PAIRS {
            let mut ids = Vec::new();
            for index in 0..105 {
                let at = now - time::Duration::days(days + if index < 50 { 2 } else { 1 });
                let id = seeds.insert(&pool, table, status, at).await?;
                ids.push((at, id));
            }
            ids.sort();
            batches.push((table, ids));
        }
        let repo = PostgresAccountDeletionRepository::new(pool.clone());
        assert_eq!(repo.purge_terminal_rows_at(now).await?.deleted, [100; 6]);
        for (table, ids) in batches {
            for (index, (_, id)) in ids.into_iter().enumerate() {
                assert_eq!(exists(&pool, table, id).await?, index >= 100);
            }
        }
        assert_eq!(repo.purge_terminal_rows_at(now).await?.deleted, [5; 6]);
        assert_eq!(repo.purge_terminal_rows_at(now).await?.deleted, [0; 6]);
        Ok(())
    }
    .await;
    finish_database_test(database, pool, result).await
}

struct ObservedRepository {
    inner: PostgresAccountDeletionRepository,
    completed: mpsc::UnboundedSender<TerminalRetentionReport>,
}

impl TerminalRetentionRepository for ObservedRepository {
    fn purge_terminal_rows(&self) -> AccountDeletionRepositoryFuture<'_, TerminalRetentionReport> {
        Box::pin(async move {
            let report = self.inner.purge_terminal_rows().await?;
            self.completed
                .send(report)
                .map_err(|_| AccountDeletionRepositoryError::Unavailable)?;
            Ok(report)
        })
    }
}

async fn run_one_logged_tick<S>(
    repository: PostgresAccountDeletionRepository,
    subscriber: S,
) -> TestResult<TerminalRetentionReport>
where
    S: tracing::Subscriber + Send + Sync + 'static,
{
    let (tick_tx, tick_rx) = mpsc::unbounded_channel();
    let ticks = stream::unfold(tick_rx, |mut rx| async move {
        rx.recv().await.map(|()| ((), rx))
    });
    let (done_tx, mut done_rx) = mpsc::unbounded_channel();
    let (stop_tx, stop_rx) = oneshot::channel();
    let repository = Arc::new(ObservedRepository {
        inner: repository,
        completed: done_tx,
    });
    let runner = tokio::spawn(
        run_terminal_retention_until(repository, ticks, async move {
            let _ = stop_rx.await;
        })
        .with_subscriber(subscriber),
    );
    tick_tx.send(())?;
    let report = done_rx
        .recv()
        .await
        .ok_or("runner stopped before completing the retention tick")?;
    stop_tx.send(()).map_err(|_| "runner already stopped")?;
    runner.await?;
    Ok(report)
}

#[tokio::test]
async fn terminal_purge_is_periodic_without_account_candidates() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let now: OffsetDateTime = sqlx::query_scalar("SELECT now()").fetch_one(&pool).await?;
        let seeds = Seeds::new(&pool).await?;
        let id = seeds
            .insert(
                &pool,
                Table::Object,
                "succeeded",
                now - time::Duration::days(91),
            )
            .await?;
        let candidates: i64 =
            sqlx::query_scalar("SELECT count(*) FROM users WHERE deleted_at IS NOT NULL")
                .fetch_one(&pool)
                .await?;
        assert_eq!(candidates, 0);
        assert!(exists(&pool, Table::Object, id).await?);
        let (tick_tx, tick_rx) = mpsc::unbounded_channel();
        let ticks = stream::unfold(tick_rx, |mut rx| async move {
            rx.recv().await.map(|()| ((), rx))
        });
        let (done_tx, mut done_rx) = mpsc::unbounded_channel();
        let (stop_tx, stop_rx) = oneshot::channel();
        let repo = Arc::new(ObservedRepository {
            inner: PostgresAccountDeletionRepository::new(pool.clone()),
            completed: done_tx,
        });
        let runner = tokio::spawn(run_terminal_retention_until(repo, ticks, async move {
            let _ = stop_rx.await;
        }));
        tick_tx.send(())?;
        assert_eq!(
            done_rx
                .recv()
                .await
                .ok_or("runner stopped before first tick")?
                .deleted,
            [0, 0, 0, 0, 0, 1]
        );
        assert!(!exists(&pool, Table::Object, id).await?);
        tick_tx.send(())?;
        assert_eq!(
            done_rx
                .recv()
                .await
                .ok_or("runner stopped before second tick")?
                .deleted,
            [0; 6]
        );
        stop_tx.send(()).map_err(|_| "runner already stopped")?;
        runner.await?;
        assert!(tick_tx.send(()).is_err());
        Ok(())
    }
    .await;
    finish_database_test(database, pool, result).await
}

#[tokio::test(flavor = "current_thread")]
async fn nonzero_success_logs_one_completion_event_with_all_counts() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let now: OffsetDateTime = sqlx::query_scalar("SELECT now()").fetch_one(&pool).await?;
        let seeds = Seeds::new(&pool).await?;
        for (index, (table, status, days)) in PAIRS.into_iter().enumerate() {
            for _ in 0..=index {
                seeds
                    .insert(&pool, table, status, now - time::Duration::days(days + 1))
                    .await?;
            }
        }
        let writer = SharedWriter::default();
        let output = writer.clone();
        let subscriber = build_json_subscriber(writer, "jamye_server=info")?;
        let _interest_sentinel = crate::logging_support::interest_sentinel();

        let report = run_one_logged_tick(
            PostgresAccountDeletionRepository::new(pool.clone()),
            subscriber,
        )
        .await?;
        assert_eq!(report.deleted, [1, 2, 3, 4, 5, 6]);
        let entries = output.parsed_lines()?;
        let completed = entries_with_message(&entries, "terminal retention batch completed");
        assert_eq!(completed.len(), 1);
        assert_completion_event(completed[0], report);
        Ok(())
    }
    .await;
    finish_database_test(database, pool, result).await
}

#[tokio::test(flavor = "current_thread")]
async fn all_zero_success_logs_no_completion_event() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let writer = SharedWriter::default();
        let output = writer.clone();
        let subscriber = build_json_subscriber(writer, "jamye_server=info")?;
        let _interest_sentinel = crate::logging_support::interest_sentinel();

        let report = run_one_logged_tick(
            PostgresAccountDeletionRepository::new(pool.clone()),
            subscriber,
        )
        .await?;
        assert_eq!(report.deleted, [0; 6]);
        let entries = output.parsed_lines()?;
        assert!(entries_with_message(&entries, "terminal retention batch completed").is_empty());
        Ok(())
    }
    .await;
    finish_database_test(database, pool, result).await
}

#[tokio::test(flavor = "current_thread")]
async fn partial_adapter_failure_logs_one_completion_event_and_delete_warning() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let now: OffsetDateTime = sqlx::query_scalar("SELECT now()").fetch_one(&pool).await?;
        let seeds = Seeds::new(&pool).await?;
        for (table, status, days) in PAIRS {
            seeds
                .insert(&pool, table, status, now - time::Duration::days(days + 1))
                .await?;
        }
        sqlx::query(
            "CREATE FUNCTION reject_published_retention_delete() RETURNS trigger \
             LANGUAGE plpgsql AS $retention$ BEGIN \
               IF OLD.status = 'published' THEN \
                 RAISE EXCEPTION USING ERRCODE = 'P0001', \
                   MESSAGE = 'test-only retention delete failure'; \
               END IF; \
               RETURN OLD; \
             END $retention$",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "CREATE TRIGGER reject_published_retention_delete \
             BEFORE DELETE ON outbox_events FOR EACH ROW \
             EXECUTE FUNCTION reject_published_retention_delete()",
        )
        .execute(&pool)
        .await?;

        let writer = SharedWriter::default();
        let output = writer.clone();
        let subscriber = build_json_subscriber(writer, "jamye_server=info")?;
        let _guard = tracing::subscriber::set_default(subscriber);
        let _interest_sentinel = crate::logging_support::interest_sentinel();
        let repository = PostgresAccountDeletionRepository::new(pool.clone());

        assert_eq!(
            repository.purge_terminal_rows_at(now).await,
            Err(AccountDeletionRepositoryError::Unavailable)
        );
        let entries = output.parsed_lines()?;
        let completed = entries_with_message(&entries, "terminal retention batch completed");
        assert_eq!(completed.len(), 1);
        assert_completion_event(
            completed[0],
            TerminalRetentionReport {
                deleted: [0, 1, 1, 1, 1, 1],
            },
        );
        let warnings = entries_with_message(&entries, "terminal retention delete failed");
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0]["target"], "jamye_server");
        assert_eq!(warnings[0]["event_kind"], "terminal_retention_batch");
        assert_eq!(warnings[0]["dependency"], "postgres");
        assert_eq!(warnings[0]["failure_kind"], "retention_delete");
        assert_eq!(warnings[0]["sqlstate"], "P0001");
        Ok(())
    }
    .await;
    finish_database_test(database, pool, result).await
}
