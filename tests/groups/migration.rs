use std::{env, fs};

use sqlx::{Connection, Row};
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

const INVITES_MIGRATION: &str = "migrations/0003_invites.sql";

#[test]
fn invite_migration_is_forward_only_and_bounds_global_usage() -> TestResult {
    let sql = fs::read_to_string(INVITES_MIGRATION)?;
    for required in [
        "-- migration: 0003_invites",
        "-- prerequisite: 0002_auth_sessions.sql",
        "-- reversibility: forward-only",
        "CREATE TABLE invites",
        "CONSTRAINT uq_invites_code UNIQUE (code)",
        "CONSTRAINT invites_max_uses_check",
        "CONSTRAINT invites_used_count_check",
        "CONSTRAINT invites_usage_bound_check",
    ] {
        assert!(sql.contains(required), "migration is missing: {required}");
    }
    assert!(!sql.contains("raw_token"));
    assert!(!sql.contains("redis"));
    Ok(())
}

#[tokio::test]
async fn invite_migration_upgrades_the_exact_0002_predecessor() -> TestResult {
    let database = TestDatabase::migrated_to(2).await?;
    let mut connection = database.connection().await?;
    assert!(!table_exists(&mut connection, "invites").await?);

    let migrator = sqlx::migrate::Migrator::new(std::path::Path::new("migrations")).await?;
    migrator.run_to(3, &mut connection).await?;
    assert!(table_exists(&mut connection, "invites").await?);
    let applied: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM _sqlx_migrations WHERE success AND version IN (1, 2, 3)",
    )
    .fetch_one(&mut connection)
    .await?;
    assert_eq!(applied, 3);

    let constraints = sqlx::query(
        "SELECT conname FROM pg_constraint WHERE conrelid = 'invites'::regclass ORDER BY conname",
    )
    .fetch_all(&mut connection)
    .await?
    .into_iter()
    .map(|row| row.try_get::<String, _>("conname"))
    .collect::<Result<Vec<_>, _>>()?;
    for constraint in [
        "invites_code_length_check",
        "invites_max_uses_check",
        "invites_usage_bound_check",
        "invites_used_count_check",
        "uq_invites_code",
    ] {
        assert!(constraints.iter().any(|name| name == constraint));
    }

    connection.close().await?;
    database.dispose().await
}

#[tokio::test]
async fn a_failed_0003_upgrade_rolls_back_the_invite_relation() -> TestResult {
    let database = TestDatabase::migrated_to(2).await?;
    let fixture_dir = env::temp_dir().join(format!(
        "jamye-server-task-6-forced-migration-{}",
        Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&fixture_dir)?;
    fs::copy(
        "migrations/0001_core_reliable_messaging.sql",
        fixture_dir.join("0001_core_reliable_messaging.sql"),
    )?;
    fs::copy(
        "migrations/0002_auth_sessions.sql",
        fixture_dir.join("0002_auth_sessions.sql"),
    )?;
    let invites_sql = fs::read_to_string(INVITES_MIGRATION)?;
    fs::write(
        fixture_dir.join("0003_forced_invites_failure.sql"),
        format!("{invites_sql}\n\nSELECT * FROM task_6_relation_that_must_not_exist;\n"),
    )?;

    let result: TestResult = async {
        let migrator = sqlx::migrate::Migrator::new(fixture_dir.as_path()).await?;
        let mut connection = database.connection().await?;
        let migration = migrator.run_to(3, &mut connection).await;
        assert!(
            migration.is_err(),
            "forced task-6 migration unexpectedly passed"
        );
        assert!(!table_exists(&mut connection, "invites").await?);
        let applied: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM _sqlx_migrations WHERE success AND version = 3",
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
