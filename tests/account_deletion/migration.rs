use std::{env, fs, io, path::Path};

use sqlx::Connection;
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

const ACCOUNT_DELETION_MIGRATION: &str = "migrations/0008_account_deletion.sql";

#[derive(Debug, Eq, PartialEq)]
struct PredecessorAccountSnapshot {
    nickname: String,
    owned_group_names: Vec<String>,
    membership_count: i64,
    message_bodies: Vec<Option<String>>,
}

#[test]
fn account_deletion_migration_is_exact_0007_forward_only_with_recovery_metadata() -> TestResult {
    let sql = migration_source()?;

    for required in [
        "-- migration: 0008_account_deletion",
        "-- prerequisite: 0007_notifications_push.sql",
        "-- reversibility: forward-only",
        "-- recovery:",
        "-- rationale:",
        "-- lock-impact:",
        "CREATE TABLE anonymous_author_tombstones",
        "CREATE TABLE account_object_deletion_intents",
        "object_key VARCHAR(512) NOT NULL",
        "claim_generation BIGINT NOT NULL DEFAULT 0",
        "lease_expires_at TIMESTAMPTZ",
        "CONSTRAINT account_object_deletion_intents_status_check",
        "CONSTRAINT account_object_deletion_intents_claim_state_check",
        "CONSTRAINT account_object_deletion_intents_terminal_state_check",
        "CREATE UNIQUE INDEX ux_account_object_deletion_intents_object_key",
        "CREATE INDEX ix_account_object_deletion_intents_due",
    ] {
        assert!(
            sql.contains(required),
            "migration is missing required account-deletion DDL or metadata: {required}"
        );
    }
    for forbidden in ["-- no-transaction", "DROP TABLE", "DROP COLUMN"] {
        assert!(
            !sql.contains(forbidden),
            "migration contains forbidden forward-only violation: {forbidden}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn account_deletion_migration_upgrades_the_exact_0007_predecessor() -> TestResult {
    migration_source()?;

    let database = TestDatabase::migrated_to(7).await?;
    let result: TestResult = async {
        let mut connection = database.connection().await?;
        for relation in [
            "anonymous_author_tombstones",
            "account_object_deletion_intents",
        ] {
            if relation_exists(&mut connection, relation).await? {
                return Err(io::Error::other(format!(
                    "exact-0007 predecessor already contains Task-11 relation: {relation}"
                ))
                .into());
            }
        }

        let migrator = sqlx::migrate::Migrator::new(Path::new("migrations")).await?;
        migrator.run_to(8, &mut connection).await?;

        for relation in [
            "anonymous_author_tombstones",
            "account_object_deletion_intents",
        ] {
            if !relation_exists(&mut connection, relation).await? {
                return Err(io::Error::other(format!(
                    "0008 upgrade did not create Task-11 relation: {relation}"
                ))
                .into());
            }
        }
        let applied: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM _sqlx_migrations WHERE success AND version BETWEEN 1 AND 8",
        )
        .fetch_one(&mut connection)
        .await?;
        if applied != 8 {
            return Err(io::Error::other(format!(
                "exact-0007 upgrade recorded {applied} successful migrations; expected 8"
            ))
            .into());
        }

        connection.close().await?;
        Ok(())
    }
    .await;

    let database_cleanup = database.dispose().await;
    match (result, database_cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(test_error), Ok(())) => Err(test_error),
        (Ok(()), Err(database_error)) => Err(database_error),
        (Err(test_error), Err(database_error)) => Err(format!(
            "migration test failed: {test_error}; database cleanup also failed: {database_error}"
        )
        .into()),
    }
}

#[tokio::test]
async fn controlled_0008_failure_rolls_back_every_account_deletion_relation() -> TestResult {
    let migration_sql = migration_source()?;
    let database = TestDatabase::migrated_to(7).await?;
    let fixture_dir = env::temp_dir().join(format!(
        "jamye-server-task-11-forced-migration-{}",
        Uuid::new_v4().simple()
    ));
    let result: TestResult = async {
        fs::create_dir_all(&fixture_dir)?;
        for migration in [
            "0001_core_reliable_messaging.sql",
            "0002_auth_sessions.sql",
            "0003_invites.sql",
            "0004_chatroom_reads.sql",
            "0005_topics.sql",
            "0006_media.sql",
            "0007_notifications_push.sql",
        ] {
            fs::copy(
                Path::new("migrations").join(migration),
                fixture_dir.join(migration),
            )?;
        }
        fs::write(
            fixture_dir.join("0008_account_deletion.sql"),
            format!("{migration_sql}\nSELECT 1 / 0;\n"),
        )?;

        let mut connection = database.connection().await?;
        let account_id = seed_exact_0007_account_content(&mut connection).await?;
        let before = predecessor_account_snapshot(&mut connection, account_id).await?;
        let migrator = sqlx::migrate::Migrator::new(fixture_dir.as_path()).await?;
        if migrator.run(&mut connection).await.is_ok() {
            return Err(io::Error::other("forced 0008 migration unexpectedly passed").into());
        }
        let after = predecessor_account_snapshot(&mut connection, account_id).await?;
        if after != before {
            return Err(io::Error::other(
                "failed 0008 changed exact-0007 account or retained-content data",
            )
            .into());
        }
        for relation in [
            "anonymous_author_tombstones",
            "account_object_deletion_intents",
        ] {
            if relation_exists(&mut connection, relation).await? {
                return Err(io::Error::other(format!(
                    "failed 0008 left a partial relation: {relation}"
                ))
                .into());
            }
        }
        let applied: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM _sqlx_migrations WHERE success AND version = 8",
        )
        .fetch_one(&mut connection)
        .await?;
        if applied != 0 {
            return Err(io::Error::other(format!(
                "failed 0008 left {applied} successful migration record(s)"
            ))
            .into());
        }

        connection.close().await?;
        Ok(())
    }
    .await;

    let fixture_cleanup = fs::remove_dir_all(&fixture_dir);
    let database_cleanup = database.dispose().await;
    match (result, fixture_cleanup, database_cleanup) {
        (Ok(()), Ok(()), Ok(())) => Ok(()),
        (Err(test_error), Ok(()), Ok(())) => Err(test_error),
        (Ok(()), Err(fixture_error), Ok(())) => Err(format!(
            "failed to remove forced-migration fixture {}: {fixture_error}",
            fixture_dir.display()
        )
        .into()),
        (Ok(()), Ok(()), Err(database_error)) => Err(database_error),
        (Err(test_error), Err(fixture_error), Ok(())) => Err(format!(
            "migration test failed: {test_error}; fixture cleanup also failed for {}: {fixture_error}",
            fixture_dir.display()
        )
        .into()),
        (Err(test_error), Ok(()), Err(database_error)) => Err(format!(
            "migration test failed: {test_error}; database cleanup also failed: {database_error}"
        )
        .into()),
        (Ok(()), Err(fixture_error), Err(database_error)) => Err(format!(
            "fixture cleanup failed for {}: {fixture_error}; database cleanup also failed: {database_error}",
            fixture_dir.display()
        )
        .into()),
        (Err(test_error), Err(fixture_error), Err(database_error)) => Err(format!(
            "migration test failed: {test_error}; fixture cleanup failed for {}: {fixture_error}; database cleanup also failed: {database_error}",
            fixture_dir.display()
        )
        .into()),
    }
}

fn migration_source() -> TestResult<String> {
    fs::read_to_string(ACCOUNT_DELETION_MIGRATION).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            std::io::Error::other(format!(
                "RED: {ACCOUNT_DELETION_MIGRATION} is absent; task-11 must add the exact-0007 forward-only account-deletion migration"
            ))
            .into()
        } else {
            error.into()
        }
    })
}

async fn seed_exact_0007_account_content(connection: &mut sqlx::PgConnection) -> TestResult<Uuid> {
    let account_id = Uuid::new_v4();
    let group_id = Uuid::new_v4();
    let chatroom_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, $2)")
        .bind(account_id)
        .bind("task-11 predecessor account")
        .execute(&mut *connection)
        .await?;
    sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, $2, $3)")
        .bind(group_id)
        .bind("task-11 predecessor group")
        .bind(account_id)
        .execute(&mut *connection)
        .await?;
    sqlx::query(
        "INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, 'owner')",
    )
    .bind(Uuid::new_v4())
    .bind(group_id)
    .bind(account_id)
    .execute(&mut *connection)
    .await?;
    sqlx::query(
        "INSERT INTO chatrooms (id, group_id, type, topic_id) VALUES ($1, $2, 'main', NULL)",
    )
    .bind(chatroom_id)
    .bind(group_id)
    .execute(&mut *connection)
    .await?;
    sqlx::query(
        "INSERT INTO messages (id, chatroom_id, sender_id, client_msg_id, body, type) \
         VALUES ($1, $2, $3, $4, $5, 'user')",
    )
    .bind(Uuid::new_v4())
    .bind(chatroom_id)
    .bind(account_id)
    .bind(Uuid::new_v4())
    .bind("task-11 predecessor retained message")
    .execute(&mut *connection)
    .await?;
    Ok(account_id)
}

async fn predecessor_account_snapshot(
    connection: &mut sqlx::PgConnection,
    account_id: Uuid,
) -> TestResult<PredecessorAccountSnapshot> {
    let nickname = sqlx::query_scalar("SELECT nickname FROM users WHERE id = $1")
        .bind(account_id)
        .fetch_one(&mut *connection)
        .await?;
    let owned_group_names =
        sqlx::query_scalar("SELECT name FROM groups WHERE owner_id = $1 ORDER BY id")
            .bind(account_id)
            .fetch_all(&mut *connection)
            .await?;
    let membership_count =
        sqlx::query_scalar("SELECT count(*) FROM memberships WHERE user_id = $1")
            .bind(account_id)
            .fetch_one(&mut *connection)
            .await?;
    let message_bodies =
        sqlx::query_scalar("SELECT body FROM messages WHERE sender_id = $1 ORDER BY id")
            .bind(account_id)
            .fetch_all(&mut *connection)
            .await?;

    Ok(PredecessorAccountSnapshot {
        nickname,
        owned_group_names,
        membership_count,
        message_bodies,
    })
}

async fn relation_exists(connection: &mut sqlx::PgConnection, relation: &str) -> TestResult<bool> {
    Ok(sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(format!("public.{relation}"))
        .fetch_one(connection)
        .await?)
}
