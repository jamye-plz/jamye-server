use std::{env, fs};

use sqlx::{Connection, Row};
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

const AUTH_MIGRATION: &str = "migrations/0002_auth_sessions.sql";

#[test]
fn auth_migration_is_forward_only_and_stores_no_raw_refresh_token() -> TestResult {
    let sql = fs::read_to_string(AUTH_MIGRATION)?;
    for required in [
        "-- migration: 0002_auth_sessions",
        "-- prerequisite: 0001_core_reliable_messaging.sql",
        "-- reversibility: forward-only",
        "CREATE TABLE auth_identities",
        "UNIQUE (provider, provider_id)",
        "CREATE TABLE refresh_sessions",
        "token_hash BYTEA NOT NULL UNIQUE",
        "parent_session_id UUID UNIQUE REFERENCES refresh_sessions (id)",
    ] {
        assert!(sql.contains(required), "migration is missing: {required}");
    }
    assert!(!sql.contains("refresh_token VARCHAR"));
    assert!(!sql.contains("access_token VARCHAR"));
    Ok(())
}

#[tokio::test]
async fn auth_migration_upgrades_the_exact_0001_predecessor() -> TestResult {
    let database = TestDatabase::migrated_to(1).await?;
    let mut connection = database.connection().await?;
    assert!(!table_exists(&mut connection, "auth_identities").await?);
    assert!(!table_exists(&mut connection, "refresh_sessions").await?);

    let migrator = sqlx::migrate::Migrator::new(std::path::Path::new("migrations")).await?;
    migrator.run_to(2, &mut connection).await?;
    assert!(table_exists(&mut connection, "auth_identities").await?);
    assert!(table_exists(&mut connection, "refresh_sessions").await?);
    let applied: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM _sqlx_migrations WHERE success AND version IN (1, 2)",
    )
    .fetch_one(&mut connection)
    .await?;
    assert_eq!(applied, 2);

    let columns = sqlx::query(
        "SELECT column_name FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'refresh_sessions' \
         ORDER BY column_name",
    )
    .fetch_all(&mut connection)
    .await?
    .into_iter()
    .map(|row| row.try_get::<String, _>("column_name"))
    .collect::<Result<Vec<_>, _>>()?;
    assert!(columns.contains(&"token_hash".to_owned()));
    assert!(!columns.contains(&"refresh_token".to_owned()));

    connection.close().await?;
    database.dispose().await
}

#[tokio::test]
async fn a_failed_0002_upgrade_rolls_back_every_auth_table() -> TestResult {
    let database = TestDatabase::migrated_to(1).await?;
    let fixture_dir = env::temp_dir().join(format!(
        "jamye-server-task-5-forced-migration-{}",
        Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&fixture_dir)?;
    fs::copy(
        "migrations/0001_core_reliable_messaging.sql",
        fixture_dir.join("0001_core_reliable_messaging.sql"),
    )?;
    let auth_sql = fs::read_to_string(AUTH_MIGRATION)?;
    fs::write(
        fixture_dir.join("0002_forced_auth_failure.sql"),
        format!("{auth_sql}\n\nSELECT * FROM task_5_relation_that_must_not_exist;\n"),
    )?;

    let result: TestResult = async {
        let migrator = sqlx::migrate::Migrator::new(fixture_dir.as_path()).await?;
        let mut connection = database.connection().await?;
        let migration = migrator.run_to(2, &mut connection).await;
        assert!(
            migration.is_err(),
            "forced task-5 migration unexpectedly passed"
        );
        assert!(!table_exists(&mut connection, "auth_identities").await?);
        assert!(!table_exists(&mut connection, "refresh_sessions").await?);
        let applied: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM _sqlx_migrations WHERE success AND version = 2",
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
