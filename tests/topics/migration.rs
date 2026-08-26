use std::{env, fs};

use sqlx::{Connection, Row};
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

const TOPICS_MIGRATION: &str = "migrations/0005_topics.sql";

#[test]
fn topic_migration_is_forward_only_and_adds_the_deferred_chatroom_fk() -> TestResult {
    let sql = fs::read_to_string(TOPICS_MIGRATION)?;
    for required in [
        "-- migration: 0005_topics",
        "-- prerequisite: 0004_chatroom_reads.sql",
        "-- reversibility: forward-only",
        "CREATE TABLE topics",
        "request_fingerprint CHAR(64) NOT NULL",
        "CONSTRAINT uq_topics_author_idempotency UNIQUE (author_id, idempotency_key)",
        "CREATE TABLE topic_media",
        "CREATE TABLE topic_tags",
        "ADD CONSTRAINT fk_chatrooms_topic_id",
        "FOREIGN KEY (topic_id) REFERENCES topics (id)",
    ] {
        assert!(sql.contains(required), "migration is missing: {required}");
    }
    assert!(!sql.contains("media_upload_id"));
    Ok(())
}

#[tokio::test]
async fn topic_migration_upgrades_the_exact_0004_predecessor() -> TestResult {
    let database = TestDatabase::migrated_to(4).await?;
    let mut connection = database.connection().await?;
    assert!(!table_exists(&mut connection, "topics").await?);

    let migrator = sqlx::migrate::Migrator::new(std::path::Path::new("migrations")).await?;
    migrator.run_to(5, &mut connection).await?;
    for table in ["topics", "topic_media", "topic_tags"] {
        assert!(table_exists(&mut connection, table).await?);
    }
    let applied: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM _sqlx_migrations WHERE success AND version BETWEEN 1 AND 5",
    )
    .fetch_one(&mut connection)
    .await?;
    assert_eq!(applied, 5);

    let foreign_keys = sqlx::query(
        "SELECT conname FROM pg_constraint \
         WHERE conrelid = 'chatrooms'::regclass AND contype = 'f' ORDER BY conname",
    )
    .fetch_all(&mut connection)
    .await?
    .into_iter()
    .map(|row| row.try_get::<String, _>("conname"))
    .collect::<Result<Vec<_>, _>>()?;
    assert!(foreign_keys.iter().any(|name| name == "fk_chatrooms_topic_id"));

    connection.close().await?;
    database.dispose().await
}

#[tokio::test]
async fn a_failed_0005_upgrade_rolls_back_every_topic_relation_and_fk() -> TestResult {
    let database = TestDatabase::migrated_to(4).await?;
    let fixture_dir = env::temp_dir().join(format!(
        "jamye-server-task-7-forced-migration-{}",
        Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&fixture_dir)?;
    for migration in [
        "0001_core_reliable_messaging.sql",
        "0002_auth_sessions.sql",
        "0003_invites.sql",
        "0004_chatroom_reads.sql",
    ] {
        fs::copy(format!("migrations/{migration}"), fixture_dir.join(migration))?;
    }
    let topics_sql = fs::read_to_string(TOPICS_MIGRATION)?;
    fs::write(
        fixture_dir.join("0005_forced_topics_failure.sql"),
        format!("{topics_sql}\n\nSELECT * FROM task_7_relation_that_must_not_exist;\n"),
    )?;

    let result: TestResult = async {
        let migrator = sqlx::migrate::Migrator::new(fixture_dir.as_path()).await?;
        let mut connection = database.connection().await?;
        let migration = migrator.run_to(5, &mut connection).await;
        assert!(migration.is_err(), "forced task-7 migration unexpectedly passed");
        for table in ["topics", "topic_media", "topic_tags"] {
            assert!(!table_exists(&mut connection, table).await?);
        }
        let fk_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'fk_chatrooms_topic_id')",
        )
        .fetch_one(&mut connection)
        .await?;
        assert!(!fk_exists);
        let applied: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM _sqlx_migrations WHERE success AND version = 5",
        )
        .fetch_one(&mut connection)
        .await?;
        assert_eq!(applied, 0);
        connection.close().await?;
        Ok(())
    }
    .await;

    let cleanup = fs::remove_dir_all(&fixture_dir);
    if let Err(error) = cleanup {
        return Err(format!("failed to remove {}: {error}", fixture_dir.display()).into());
    }
    result?;
    database.dispose().await
}

async fn table_exists(connection: &mut sqlx::PgConnection, table: &str) -> TestResult<bool> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS ( \
             SELECT 1 FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_name = $1 \
         )",
    )
    .bind(table)
    .fetch_one(connection)
    .await?)
}
