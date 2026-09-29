use std::{env, fs, path::Path};

use sqlx::{Connection, Row};
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

const AUTH_MIGRATION: &str = "migrations/0002_auth_sessions.sql";
const APPLE_PROVIDER_MIGRATION: &str = "migrations/0018_add_apple_auth_identity_provider.sql";

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

#[test]
fn migration_0018_only_extends_the_auth_identity_provider_check() -> TestResult {
    let sql = fs::read_to_string(APPLE_PROVIDER_MIGRATION)?;
    for required in [
        "-- migration: 0018_add_apple_auth_identity_provider",
        "-- prerequisite: 0017_remaining_soft_delete_live_uniqueness.sql",
        "-- reversibility: forward-only",
        "-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md",
        "-- rationale:",
        "-- lock impact:",
        "SET LOCAL lock_timeout = '5s';",
        "SET LOCAL statement_timeout = '120s';",
        "DROP CONSTRAINT auth_identities_provider_check",
        "CHECK (provider IN ('kakao', 'google', 'apple'))",
    ] {
        assert!(
            sql.contains(required),
            "0018 migration is missing: {required}"
        );
    }
    for forbidden in [
        "-- no-transaction",
        "CREATE TABLE",
        "ALTER TABLE users",
        "ADD COLUMN",
        "CREATE INDEX",
        "INSERT INTO",
        "UPDATE ",
    ] {
        assert!(
            !sql.contains(forbidden),
            "0018 migration changed more than the provider CHECK: {forbidden}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn migration_0018_upgrades_0017_and_accepts_only_the_new_apple_provider() -> TestResult {
    let database = TestDatabase::migrated_to(17).await?;
    let mut connection = database.connection().await?;
    let user_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, 'apple migration')")
        .bind(user_id)
        .execute(&mut connection)
        .await?;
    assert!(
        insert_identity(&mut connection, user_id, "apple")
            .await
            .is_err(),
        "exact 0017 predecessor unexpectedly accepted apple"
    );

    let migrator = sqlx::migrate::Migrator::new(Path::new("migrations")).await?;
    migrator.run_to(18, &mut connection).await?;
    insert_identity(&mut connection, user_id, "apple").await?;
    assert!(
        insert_identity(&mut connection, user_id, "password")
            .await
            .is_err(),
        "0018 accepted an unknown provider"
    );
    let applied: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM _sqlx_migrations WHERE success AND version BETWEEN 1 AND 18",
    )
    .fetch_one(&mut connection)
    .await?;
    assert_eq!(applied, 18);

    connection.close().await?;
    database.dispose().await
}

#[tokio::test]
async fn a_failed_0018_upgrade_rolls_back_the_provider_check_change() -> TestResult {
    let database = TestDatabase::migrated_to(17).await?;
    let fixture_dir = env::temp_dir().join(format!(
        "jamye-server-task-15-forced-migration-{}",
        Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&fixture_dir)?;
    for entry in fs::read_dir("migrations")? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| std::io::Error::other("migration filename is not UTF-8"))?;
        if name.ends_with(".sql") && name < "0018_add_apple_auth_identity_provider.sql" {
            fs::copy(entry.path(), fixture_dir.join(name))?;
        }
    }
    let apple_sql = fs::read_to_string(APPLE_PROVIDER_MIGRATION)?;
    fs::write(
        fixture_dir.join("0018_add_apple_auth_identity_provider.sql"),
        format!("{apple_sql}\n\nSELECT * FROM task_15_relation_that_must_not_exist;\n"),
    )?;

    let result: TestResult = async {
        let migrator = sqlx::migrate::Migrator::new(fixture_dir.as_path()).await?;
        let mut connection = database.connection().await?;
        let user_id = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, 'apple rollback')")
            .bind(user_id)
            .execute(&mut connection)
            .await?;
        assert!(migrator.run_to(18, &mut connection).await.is_err());
        assert!(
            insert_identity(&mut connection, user_id, "apple")
                .await
                .is_err(),
            "failed 0018 left apple accepted by the provider CHECK"
        );
        let applied: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM _sqlx_migrations WHERE success AND version = 18",
        )
        .fetch_one(&mut connection)
        .await?;
        assert_eq!(applied, 0);
        connection.close().await?;
        Ok(())
    }
    .await;

    fs::remove_dir_all(&fixture_dir)?;
    result?;
    database.dispose().await
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

async fn insert_identity(
    connection: &mut sqlx::PgConnection,
    user_id: Uuid,
    provider: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO auth_identities (id, user_id, provider, provider_id) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(provider)
    .bind(format!("{provider}-subject"))
    .execute(connection)
    .await
    .map(|_| ())
}
