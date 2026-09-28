//! Fresh disposable migration-chain coverage owned by Task-12.

use std::{env, fs, io, path::Path};

use sqlx::{Connection, PgConnection};
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

const APPLICATION_TABLES: &[&str] = &[
    "users",
    "groups",
    "memberships",
    "chatrooms",
    "messages",
    "conversation_events",
    "outbox_events",
    "auth_identities",
    "refresh_sessions",
    "invites",
    "chatroom_reads",
    "topics",
    "topic_tags",
    "media_uploads",
    "message_media",
    "notifications",
    "push_installations",
    "push_delivery_intents",
    "anonymous_author_tombstones",
    "account_object_deletion_intents",
];

const MIGRATIONS_0001_THROUGH_0013: &[&str] = &[
    "0001_core_reliable_messaging.sql",
    "0002_auth_sessions.sql",
    "0003_invites.sql",
    "0004_chatroom_reads.sql",
    "0005_topics.sql",
    "0006_media.sql",
    "0007_notifications_push.sql",
    "0008_account_deletion.sql",
    "0009_message_anchor_index.sql",
    "0010_media_posters.sql",
    "0011_main_chat_notifications.sql",
    "0012_remove_topic_media.sql",
    "0013_https_avatar_urls.sql",
];

const MIGRATIONS_0001_THROUGH_0014: &[&str] = &[
    "0001_core_reliable_messaging.sql",
    "0002_auth_sessions.sql",
    "0003_invites.sql",
    "0004_chatroom_reads.sql",
    "0005_topics.sql",
    "0006_media.sql",
    "0007_notifications_push.sql",
    "0008_account_deletion.sql",
    "0009_message_anchor_index.sql",
    "0010_media_posters.sql",
    "0011_main_chat_notifications.sql",
    "0012_remove_topic_media.sql",
    "0013_https_avatar_urls.sql",
    "0014_audit_columns_and_updated_at_triggers.sql",
];

const MIGRATIONS_0001_THROUGH_0015: &[&str] = &[
    "0001_core_reliable_messaging.sql",
    "0002_auth_sessions.sql",
    "0003_invites.sql",
    "0004_chatroom_reads.sql",
    "0005_topics.sql",
    "0006_media.sql",
    "0007_notifications_push.sql",
    "0008_account_deletion.sql",
    "0009_message_anchor_index.sql",
    "0010_media_posters.sql",
    "0011_main_chat_notifications.sql",
    "0012_remove_topic_media.sql",
    "0013_https_avatar_urls.sql",
    "0014_audit_columns_and_updated_at_triggers.sql",
    "0015_delete_events_and_live_read_indexes.sql",
];

#[tokio::test]
async fn fresh_disposable_database_applies_the_canonical_0001_through_0016_chain() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let mut connection = database.connection().await?;
    let result: TestResult = async {
        let applied: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM _sqlx_migrations WHERE success AND version BETWEEN 1 AND 16",
        )
        .fetch_one(&mut connection)
        .await?;
        let latest: i64 = sqlx::query_scalar(
            "SELECT coalesce(max(version), 0) FROM _sqlx_migrations WHERE success",
        )
        .fetch_one(&mut connection)
        .await?;
        require_eq(
            applied,
            16,
            "fresh disposable chain did not apply 0001 through 0016",
        )?;
        require_eq(latest, 16, "fresh disposable chain did not end at 0016")?;
        Ok(())
    }
    .await;

    connection.close().await?;
    database.dispose().await?;
    result
}

#[test]
fn migration_0014_is_forward_only_audit_columns_and_trigger_metadata() -> TestResult {
    let sql = migration_0014_source()?;
    for required in [
        "-- migration: 0014_audit_columns_and_updated_at_triggers",
        "-- prerequisite: 0013_https_avatar_urls.sql",
        "-- reversibility: forward-only",
        "-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md",
        "-- rationale:",
        "-- lock impact:",
        "CREATE OR REPLACE FUNCTION jamye_set_updated_at_if_changed()",
        "NEW IS DISTINCT FROM OLD",
        "NEW.updated_at IS NOT DISTINCT FROM OLD.updated_at",
        "CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON users",
    ] {
        require(
            sql.contains(required),
            &format!("0014 migration is missing required metadata or D19 DDL: {required}"),
        )?;
    }
    for forbidden in [
        "-- no-transaction",
        "DROP TABLE",
        "DROP COLUMN",
        "CREATE UNIQUE INDEX",
        "WHERE deleted_at IS NULL",
    ] {
        require(
            !sql.contains(forbidden),
            &format!("0014 migration crossed into delete/grace/14b scope: {forbidden}"),
        )?;
    }
    Ok(())
}

#[tokio::test]
async fn migration_0014_upgrades_0013_with_audit_columns_backfill_and_triggers() -> TestResult {
    let database = TestDatabase::migrated_to(13).await?;
    let mut connection = database.connection().await?;
    let result: TestResult = async {
        require(
            !column_exists(&mut connection, "users", "updated_at").await?,
            "exact 0013 predecessor already contained users.updated_at",
        )?;
        let user_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, nickname, created_at) \
             VALUES ($1, 'audit predecessor', '2026-01-01T00:00:00Z')",
        )
        .bind(user_id)
        .execute(&mut connection)
        .await?;

        let migrator = sqlx::migrate::Migrator::new(Path::new("migrations")).await?;
        migrator.run_to(14, &mut connection).await?;

        for table in APPLICATION_TABLES {
            for column in ["created_at", "updated_at", "deleted_at"] {
                require(
                    column_exists(&mut connection, table, column).await?,
                    &format!("0014 did not add {table}.{column}"),
                )?;
            }
        }
        require_eq(
            trigger_count(&mut connection).await?,
            i64::try_from(APPLICATION_TABLES.len())?,
            "0014 did not install one D19 trigger per application table",
        )?;
        require(
            user_updated_at_was_backfilled(&mut connection, user_id).await?,
            "0014 did not backfill users.updated_at from created_at",
        )?;
        assert_updated_at_trigger_behavior(&mut connection, user_id).await?;
        assert_trigger_order_allows_test_failure_injection(&mut connection).await?;
        Ok(())
    }
    .await;

    connection.close().await?;
    database.dispose().await?;
    result
}

#[tokio::test]
async fn controlled_0014_failure_rolls_back_audit_columns_and_trigger_function() -> TestResult {
    let migration_sql = migration_0014_source()?;
    let database = TestDatabase::migrated_to(13).await?;
    let fixture_dir = env::temp_dir().join(format!(
        "jamye-server-task-14a-forced-migration-{}",
        Uuid::new_v4().simple()
    ));
    let result: TestResult = async {
        fs::create_dir_all(&fixture_dir)?;
        for migration in MIGRATIONS_0001_THROUGH_0013 {
            fs::copy(
                Path::new("migrations").join(migration),
                fixture_dir.join(migration),
            )?;
        }
        fs::write(
            fixture_dir.join("0014_audit_columns_and_updated_at_triggers.sql"),
            format!("{migration_sql}\nSELECT 1 / 0;\n"),
        )?;

        let mut connection = database.connection().await?;
        let migrator = sqlx::migrate::Migrator::new(fixture_dir.as_path()).await?;
        require(
            migrator.run(&mut connection).await.is_err(),
            "forced 0014 migration unexpectedly passed",
        )?;
        require(
            !column_exists(&mut connection, "users", "updated_at").await?,
            "failed 0014 left users.updated_at behind",
        )?;
        require(
            trigger_function_absent(&mut connection).await?,
            "failed 0014 left jamye_set_updated_at_if_changed behind",
        )?;
        let applied: i64 =
            sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version = 14")
                .fetch_one(&mut connection)
                .await?;
        require_eq(applied, 0, "failed 0014 recorded a successful migration")?;
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
            "failed to remove forced 0014 fixture {}: {fixture_error}",
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

#[test]
fn migration_0015_is_forward_only_delete_event_live_read_metadata() -> TestResult {
    let sql = migration_0015_source()?;
    for required in [
        "-- migration: 0015_delete_events_and_live_read_indexes",
        "-- prerequisite: 0014_audit_columns_and_updated_at_triggers.sql",
        "-- reversibility: forward-only",
        "-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md",
        "-- rationale:",
        "-- lock impact:",
        "DROP CONSTRAINT topics_title_check",
        "deleted_at IS NOT NULL",
        "CREATE INDEX ix_messages_chatroom_created_live",
        "WHERE deleted_at IS NULL",
    ] {
        require(
            sql.contains(required),
            &format!("0015 migration is missing required delete metadata or DDL: {required}"),
        )?;
    }
    for forbidden in ["-- no-transaction", "DROP TABLE", "DROP COLUMN"] {
        require(
            !sql.contains(forbidden),
            &format!("0015 migration crossed destructive migration scope: {forbidden}"),
        )?;
    }
    Ok(())
}

#[tokio::test]
async fn migration_0015_upgrades_0014_with_deleted_topic_title_scrub_constraint() -> TestResult {
    let database = TestDatabase::migrated_to(14).await?;
    let mut connection = database.connection().await?;
    let result: TestResult = async {
        require(
            !topic_title_constraint_mentions_deleted_at(&mut connection).await?,
            "exact 0014 predecessor already had the 0015 topic title scrub constraint",
        )?;
        let user_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let topic_id = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, 'topic scrub author')")
            .bind(user_id)
            .execute(&mut connection)
            .await?;
        sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, 'topic scrub group', $2)")
            .bind(group_id)
            .bind(user_id)
            .execute(&mut connection)
            .await?;
        sqlx::query(
            "INSERT INTO topics \
                 (id, group_id, author_id, idempotency_key, request_fingerprint, title) \
             VALUES ($1, $2, $3, $4, $5, 'scrubbable title')",
        )
        .bind(topic_id)
        .bind(group_id)
        .bind(user_id)
        .bind(Uuid::new_v4())
        .bind("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        .execute(&mut connection)
        .await?;

        let migrator = sqlx::migrate::Migrator::new(Path::new("migrations")).await?;
        migrator.run_to(15, &mut connection).await?;

        require(
            topic_title_constraint_mentions_deleted_at(&mut connection).await?,
            "0015 did not replace topics_title_check with delete-aware predicate",
        )?;
        sqlx::query("UPDATE topics SET deleted_at = clock_timestamp(), title = '' WHERE id = $1")
            .bind(topic_id)
            .execute(&mut connection)
            .await?;
        let live_blank_insert = sqlx::query(
            "INSERT INTO topics \
                 (id, group_id, author_id, idempotency_key, request_fingerprint, title) \
             VALUES ($1, $2, $3, $4, $5, '')",
        )
        .bind(Uuid::new_v4())
        .bind(group_id)
        .bind(user_id)
        .bind(Uuid::new_v4())
        .bind("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
        .execute(&mut connection)
        .await;
        require(
            live_blank_insert.is_err(),
            "0015 allowed a live topic with an empty title",
        )?;
        Ok(())
    }
    .await;

    connection.close().await?;
    database.dispose().await?;
    result
}

#[tokio::test]
async fn controlled_0015_failure_rolls_back_delete_constraints_and_indexes() -> TestResult {
    let migration_sql = migration_0015_source()?;
    let database = TestDatabase::migrated_to(14).await?;
    let fixture_dir = env::temp_dir().join(format!(
        "jamye-server-task-14a-delete-forced-migration-{}",
        Uuid::new_v4().simple()
    ));
    let result: TestResult = async {
        fs::create_dir_all(&fixture_dir)?;
        for migration in MIGRATIONS_0001_THROUGH_0014 {
            fs::copy(
                Path::new("migrations").join(migration),
                fixture_dir.join(migration),
            )?;
        }
        fs::write(
            fixture_dir.join("0015_delete_events_and_live_read_indexes.sql"),
            format!("{migration_sql}\nSELECT 1 / 0;\n"),
        )?;

        let mut connection = database.connection().await?;
        let migrator = sqlx::migrate::Migrator::new(fixture_dir.as_path()).await?;
        require(
            migrator.run(&mut connection).await.is_err(),
            "forced 0015 migration unexpectedly passed",
        )?;
        require(
            !topic_title_constraint_mentions_deleted_at(&mut connection).await?,
            "failed 0015 left delete-aware topics_title_check behind",
        )?;
        require(
            !index_exists(&mut connection, "ix_messages_chatroom_created_live").await?,
            "failed 0015 left live message index behind",
        )?;
        let applied: i64 =
            sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version = 15")
                .fetch_one(&mut connection)
                .await?;
        require_eq(applied, 0, "failed 0015 recorded a successful migration")?;
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
            "failed to remove forced 0015 fixture {}: {fixture_error}",
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

#[test]
fn migration_0016_is_forward_only_account_grace_metadata() -> TestResult {
    let sql = migration_0016_source()?;
    for required in [
        "-- migration: 0016_account_grace_period",
        "-- prerequisite: 0015_delete_events_and_live_read_indexes.sql",
        "-- reversibility: forward-only",
        "-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md",
        "-- rationale:",
        "-- lock impact:",
        "ADD COLUMN account_deleted_at TIMESTAMPTZ",
        "CREATE UNIQUE INDEX uq_memberships_group_user",
        "WHERE deleted_at IS NULL",
        "CREATE UNIQUE INDEX uq_auth_identities_provider_principal",
        "CREATE INDEX ix_users_account_purge_due",
    ] {
        require(
            sql.contains(required),
            &format!("0016 migration is missing account-grace metadata or DDL: {required}"),
        )?;
    }
    for forbidden in ["-- no-transaction", "DROP TABLE", "DROP COLUMN"] {
        require(
            !sql.contains(forbidden),
            &format!("0016 migration crossed destructive migration scope: {forbidden}"),
        )?;
    }
    Ok(())
}

#[tokio::test]
async fn migration_0016_upgrades_0015_with_live_row_uniqueness_and_purge_claims() -> TestResult {
    let database = TestDatabase::migrated_to(15).await?;
    let mut connection = database.connection().await?;
    let result: TestResult = async {
        require(
            !column_exists(&mut connection, "memberships", "account_deleted_at").await?,
            "exact 0015 predecessor already had memberships.account_deleted_at",
        )?;
        let user_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, 'grace predecessor')")
            .bind(user_id)
            .execute(&mut connection)
            .await?;
        sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, 'grace group', $2)")
            .bind(group_id)
            .bind(user_id)
            .execute(&mut connection)
            .await?;
        sqlx::query(
            "INSERT INTO memberships (id, group_id, user_id, role) \
             VALUES ($1, $2, $3, 'owner')",
        )
        .bind(Uuid::new_v4())
        .bind(group_id)
        .bind(user_id)
        .execute(&mut connection)
        .await?;
        sqlx::query(
            "INSERT INTO auth_identities (id, user_id, provider, provider_id) \
             VALUES ($1, $2, 'kakao', 'grace-principal')",
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .execute(&mut connection)
        .await?;

        let migrator = sqlx::migrate::Migrator::new(Path::new("migrations")).await?;
        migrator.run_to(16, &mut connection).await?;

        require(
            column_exists(&mut connection, "memberships", "account_deleted_at").await?,
            "0016 did not add memberships.account_deleted_at",
        )?;
        require(
            column_exists(&mut connection, "users", "account_purge_claim_owner").await?,
            "0016 did not add user purge-claim columns",
        )?;
        require(
            index_predicate_mentions_deleted_at(&mut connection, "uq_memberships_group_user")
                .await?,
            "0016 did not convert memberships uniqueness to a live-row partial index",
        )?;
        require(
            index_predicate_mentions_deleted_at(
                &mut connection,
                "uq_auth_identities_provider_principal",
            )
            .await?,
            "0016 did not convert auth identity uniqueness to a live-row partial index",
        )?;

        sqlx::query(
            "UPDATE memberships \
             SET deleted_at = clock_timestamp(), account_deleted_at = clock_timestamp() \
             WHERE group_id = $1 AND user_id = $2",
        )
        .bind(group_id)
        .bind(user_id)
        .execute(&mut connection)
        .await?;
        sqlx::query(
            "INSERT INTO memberships (id, group_id, user_id, role) \
             VALUES ($1, $2, $3, 'member')",
        )
        .bind(Uuid::new_v4())
        .bind(group_id)
        .bind(user_id)
        .execute(&mut connection)
        .await?;

        sqlx::query(
            "UPDATE auth_identities \
             SET deleted_at = clock_timestamp() \
             WHERE provider = 'kakao' AND provider_id = 'grace-principal'",
        )
        .execute(&mut connection)
        .await?;
        sqlx::query(
            "INSERT INTO auth_identities (id, user_id, provider, provider_id) \
             VALUES ($1, $2, 'kakao', 'grace-principal')",
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .execute(&mut connection)
        .await?;
        Ok(())
    }
    .await;

    connection.close().await?;
    database.dispose().await?;
    result
}

#[tokio::test]
async fn controlled_0016_failure_rolls_back_account_grace_schema() -> TestResult {
    let migration_sql = migration_0016_source()?;
    let database = TestDatabase::migrated_to(15).await?;
    let fixture_dir = env::temp_dir().join(format!(
        "jamye-server-task-14a-grace-forced-migration-{}",
        Uuid::new_v4().simple()
    ));
    let result: TestResult = async {
        fs::create_dir_all(&fixture_dir)?;
        for migration in MIGRATIONS_0001_THROUGH_0015 {
            fs::copy(
                Path::new("migrations").join(migration),
                fixture_dir.join(migration),
            )?;
        }
        fs::write(
            fixture_dir.join("0016_account_grace_period.sql"),
            format!("{migration_sql}\nSELECT 1 / 0;\n"),
        )?;

        let mut connection = database.connection().await?;
        let migrator = sqlx::migrate::Migrator::new(fixture_dir.as_path()).await?;
        require(
            migrator.run(&mut connection).await.is_err(),
            "forced 0016 migration unexpectedly passed",
        )?;
        require(
            !column_exists(&mut connection, "memberships", "account_deleted_at").await?,
            "failed 0016 left memberships.account_deleted_at behind",
        )?;
        require(
            !column_exists(&mut connection, "users", "account_purge_claim_owner").await?,
            "failed 0016 left users.account_purge_claim_owner behind",
        )?;
        let applied: i64 =
            sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version = 16")
                .fetch_one(&mut connection)
                .await?;
        require_eq(applied, 0, "failed 0016 recorded a successful migration")?;
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
            "failed to remove forced 0016 fixture {}: {fixture_error}",
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

#[tokio::test]
async fn migration_0005_adds_chatrooms_topic_id_fk_only_after_topics_exists() -> TestResult {
    let database = TestDatabase::migrated_to(4).await?;
    let mut connection = database.connection().await?;
    let result: TestResult = async {
        require(
            !table_exists(&mut connection, "topics").await?,
            "exact 0004 predecessor unexpectedly contained topics",
        )?;
        require(
            column_exists(&mut connection, "chatrooms", "topic_id").await?,
            "exact 0004 predecessor did not contain chatrooms.topic_id",
        )?;
        require_eq(
            chatrooms_topic_fk_count(&mut connection).await?,
            0,
            "exact 0004 predecessor already contained the topics foreign key",
        )?;

        let migrator = sqlx::migrate::Migrator::new(std::path::Path::new("migrations")).await?;
        migrator.run_to(5, &mut connection).await?;

        require(
            table_exists(&mut connection, "topics").await?,
            "0005 did not create topics",
        )?;
        require_eq(
            chatrooms_topic_fk_count(&mut connection).await?,
            1,
            "0005 did not add exactly one chatrooms.topic_id -> topics(id) FK after topics existed",
        )?;
        Ok(())
    }
    .await;

    connection.close().await?;
    database.dispose().await?;
    result
}

async fn table_exists(connection: &mut PgConnection, table: &str) -> TestResult<bool> {
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

async fn column_exists(
    connection: &mut PgConnection,
    table: &str,
    column: &str,
) -> TestResult<bool> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS ( \
             SELECT 1 FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name = $1 AND column_name = $2 \
         )",
    )
    .bind(table)
    .bind(column)
    .fetch_one(connection)
    .await?)
}

async fn chatrooms_topic_fk_count(connection: &mut PgConnection) -> TestResult<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM pg_constraint c \
             WHERE c.conrelid = to_regclass('public.chatrooms') \
               AND c.contype = 'f' \
               AND c.confrelid = to_regclass('public.topics') \
               AND c.conkey = ARRAY[ \
                   (SELECT attnum FROM pg_attribute \
                    WHERE attrelid = to_regclass('public.chatrooms') \
                      AND attname = 'topic_id' \
                      AND NOT attisdropped) \
               ]::smallint[] \
               AND c.confkey = ARRAY[ \
                   (SELECT attnum FROM pg_attribute \
                    WHERE attrelid = to_regclass('public.topics') \
                      AND attname = 'id' \
                      AND NOT attisdropped) \
               ]::smallint[]",
    )
    .fetch_one(connection)
    .await?)
}

fn migration_0014_source() -> TestResult<String> {
    fs::read_to_string("migrations/0014_audit_columns_and_updated_at_triggers.sql").map_err(
        |error| {
            if error.kind() == io::ErrorKind::NotFound {
                io::Error::other(
                    "RED: migrations/0014_audit_columns_and_updated_at_triggers.sql is absent",
                )
                .into()
            } else {
                error.into()
            }
        },
    )
}

fn migration_0015_source() -> TestResult<String> {
    fs::read_to_string("migrations/0015_delete_events_and_live_read_indexes.sql").map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            io::Error::other(
                "RED: migrations/0015_delete_events_and_live_read_indexes.sql is absent",
            )
            .into()
        } else {
            error.into()
        }
    })
}

fn migration_0016_source() -> TestResult<String> {
    fs::read_to_string("migrations/0016_account_grace_period.sql").map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            io::Error::other("RED: migrations/0016_account_grace_period.sql is absent").into()
        } else {
            error.into()
        }
    })
}

async fn topic_title_constraint_mentions_deleted_at(
    connection: &mut PgConnection,
) -> TestResult<bool> {
    let definition: Option<String> = sqlx::query_scalar(
        "SELECT pg_get_constraintdef(oid) \
         FROM pg_constraint \
         WHERE conrelid = 'topics'::regclass \
           AND conname = 'topics_title_check'",
    )
    .fetch_optional(connection)
    .await?;
    Ok(definition.is_some_and(|definition| definition.contains("deleted_at")))
}

async fn index_exists(connection: &mut PgConnection, index_name: &str) -> TestResult<bool> {
    Ok(
        sqlx::query_scalar::<_, bool>("SELECT to_regclass(format('public.%I', $1)) IS NOT NULL")
            .bind(index_name)
            .fetch_one(connection)
            .await?,
    )
}

async fn index_predicate_mentions_deleted_at(
    connection: &mut PgConnection,
    index_name: &str,
) -> TestResult<bool> {
    let predicate: Option<String> = sqlx::query_scalar(
        "SELECT pg_get_expr(index_entry.indpred, index_entry.indrelid) \
         FROM pg_index index_entry \
         JOIN pg_class index_class ON index_class.oid = index_entry.indexrelid \
         WHERE index_class.relname = $1",
    )
    .bind(index_name)
    .fetch_optional(connection)
    .await?;
    Ok(predicate.is_some_and(|predicate| predicate.contains("deleted_at IS NULL")))
}

async fn trigger_count(connection: &mut PgConnection) -> TestResult<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT count(*) \
         FROM pg_trigger \
         WHERE tgname = 'trg_jamye_set_updated_at' \
           AND NOT tgisinternal",
    )
    .fetch_one(connection)
    .await?)
}

async fn user_updated_at_was_backfilled(
    connection: &mut PgConnection,
    user_id: Uuid,
) -> TestResult<bool> {
    Ok(
        sqlx::query_scalar::<_, bool>("SELECT updated_at = created_at FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(connection)
            .await?,
    )
}

async fn assert_updated_at_trigger_behavior(
    connection: &mut PgConnection,
    user_id: Uuid,
) -> TestResult {
    let initial = user_updated_at(connection, user_id).await?;
    sqlx::query("UPDATE users SET nickname = nickname WHERE id = $1")
        .bind(user_id)
        .execute(&mut *connection)
        .await?;
    require_eq(
        user_updated_at(connection, user_id).await?,
        initial,
        "D19 trigger changed updated_at for a no-op UPDATE",
    )?;

    sqlx::query("UPDATE users SET nickname = 'audit changed' WHERE id = $1")
        .bind(user_id)
        .execute(&mut *connection)
        .await?;
    let changed = user_updated_at(connection, user_id).await?;
    require(
        changed > initial,
        "D19 trigger did not advance updated_at when a row changed",
    )?;

    sqlx::query(
        "UPDATE users \
         SET nickname = 'audit direct', updated_at = '2026-01-02T03:04:05Z' \
         WHERE id = $1",
    )
    .bind(user_id)
    .execute(&mut *connection)
    .await?;
    let direct: time::OffsetDateTime = time::OffsetDateTime::parse(
        "2026-01-02T03:04:05Z",
        &time::format_description::well_known::Rfc3339,
    )?;
    require_eq(
        user_updated_at(connection, user_id).await?,
        direct,
        "D19 trigger overwrote a caller-supplied updated_at",
    )
}

async fn assert_trigger_order_allows_test_failure_injection(
    connection: &mut PgConnection,
) -> TestResult {
    sqlx::query(
        "CREATE FUNCTION zz_task14_fail_after_audit_trigger() \
         RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'task-14a trigger order sentinel'; END $$",
    )
    .execute(&mut *connection)
    .await?;
    sqlx::query(
        "CREATE TRIGGER zz_task14_fail_after_audit_trigger \
         BEFORE UPDATE ON users \
         FOR EACH ROW EXECUTE FUNCTION zz_task14_fail_after_audit_trigger()",
    )
    .execute(&mut *connection)
    .await?;
    let names = sqlx::query_scalar::<_, String>(
        "SELECT tgname \
         FROM pg_trigger \
         WHERE tgrelid = 'users'::regclass \
           AND NOT tgisinternal \
           AND tgname IN ('trg_jamye_set_updated_at', 'zz_task14_fail_after_audit_trigger') \
         ORDER BY tgname",
    )
    .fetch_all(&mut *connection)
    .await?;
    require_eq(
        names,
        vec![
            "trg_jamye_set_updated_at".to_owned(),
            "zz_task14_fail_after_audit_trigger".to_owned(),
        ],
        "D19 trigger name no longer sorts before zz_ failure-injection triggers",
    )?;
    Ok(())
}

async fn user_updated_at(
    connection: &mut PgConnection,
    user_id: Uuid,
) -> TestResult<time::OffsetDateTime> {
    Ok(
        sqlx::query_scalar("SELECT updated_at FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(connection)
            .await?,
    )
}

async fn trigger_function_absent(connection: &mut PgConnection) -> TestResult<bool> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT to_regprocedure('jamye_set_updated_at_if_changed()') IS NULL",
    )
    .fetch_one(connection)
    .await?)
}

fn require(condition: bool, message: &str) -> TestResult {
    condition
        .then_some(())
        .ok_or_else(|| io::Error::other(message).into())
}

fn require_eq<T>(actual: T, expected: T, message: &str) -> TestResult
where
    T: std::fmt::Debug + PartialEq,
{
    if actual == expected {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{message}: actual={actual:?}, expected={expected:?}"
        ))
        .into())
    }
}
