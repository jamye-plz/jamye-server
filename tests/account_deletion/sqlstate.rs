use jamye_server::{
    adapters::postgres::moderation::PostgresModerationRepository,
    platform::logging::build_json_subscriber,
    ports::moderation::{ModerationRepository, ModerationRepositoryError},
};
use sqlx::{Connection, postgres::PgPoolOptions};
use uuid::Uuid;

use crate::{
    TestResult, logging_support::SharedWriter, postgres_support::TestDatabase,
    support::finish_database_test,
};

#[tokio::test(flavor = "current_thread")]
async fn statement_timeout_logs_sqlstate_without_query_text() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy(database.database_url())?;
    let result: TestResult = async {
        let mut lock_connection = database.connection().await?;
        let mut lock = lock_connection.begin().await?;
        sqlx::query("LOCK TABLE user_blocks IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *lock)
            .await?;
        {
            let mut connection = pool.acquire().await?;
            sqlx::query("SET lock_timeout = 0")
                .execute(&mut *connection)
                .await?;
            sqlx::query("SET statement_timeout = '100ms'")
                .execute(&mut *connection)
                .await?;
        }

        let writer = SharedWriter::default();
        let output = writer.clone();
        let subscriber = build_json_subscriber(writer, "jamye_server=warn")?;
        let _guard = tracing::subscriber::set_default(subscriber);
        let _interest_sentinel = crate::logging_support::interest_sentinel();
        let repository = PostgresModerationRepository::new(pool.clone());
        let blocker_id = Uuid::new_v4();
        assert!(matches!(
            repository.list_blocks(blocker_id).await,
            Err(ModerationRepositoryError::Unavailable)
        ));
        let entries = output.parsed_lines()?;
        let warnings: Vec<_> = entries
            .iter()
            .filter(|entry| entry["message"] == "PostgreSQL moderation operation failed")
            .collect();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0]["operation"], "moderation_list_blocks");
        assert_eq!(warnings[0]["sqlstate"], "57014");
        let logs = output.snapshot()?;
        assert!(!logs.contains("SELECT ub.blocked_id"));
        assert!(!logs.contains("user_blocks"));
        assert!(!logs.contains("canceling statement"));
        assert!(!logs.contains(&blocker_id.to_string()));
        lock.rollback().await?;
        Ok(())
    }
    .await;
    finish_database_test(database, pool, result).await
}
