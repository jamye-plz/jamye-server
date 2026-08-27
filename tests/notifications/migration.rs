use std::{env, fs, io};

use sqlx::Connection;
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

const NOTIFICATIONS_MIGRATION: &str = "migrations/0007_notifications_push.sql";

#[test]
fn notifications_push_migration_is_forward_only_and_owns_canonical_occurrence_state() -> TestResult
{
    let sql = fs::read_to_string(NOTIFICATIONS_MIGRATION).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            io::Error::other(format!(
                "RED: {NOTIFICATIONS_MIGRATION} is absent; task-9 must add the canonical notification, Expo installation, and per-source-event push occurrence schema"
            ))
        } else {
            error
        }
    })?;

    for required in [
        "-- migration: 0007_notifications_push",
        "-- prerequisite: 0006_media.sql",
        "-- reversibility: forward-only",
        "CREATE TABLE notifications",
        "CONSTRAINT notifications_type_check",
        "CONSTRAINT notifications_payload_check",
        "CREATE UNIQUE INDEX ux_notifications_user_dedup",
        "CREATE TABLE push_installations",
        "owner_epoch BIGINT NOT NULL DEFAULT 1",
        "provider VARCHAR(8) NOT NULL DEFAULT 'expo'",
        "message_preview_enabled BOOLEAN NOT NULL DEFAULT false",
        "CONSTRAINT uq_push_installations_installation_id UNIQUE (installation_id)",
        "CONSTRAINT uq_push_installations_destination UNIQUE (environment, token)",
        "CREATE TABLE push_delivery_intents",
        "notification_id UUID NOT NULL REFERENCES notifications (id)",
        "source_event_id UUID NOT NULL REFERENCES conversation_events (id)",
        "source_message_id UUID REFERENCES messages (id)",
        "recipient_user_id UUID NOT NULL REFERENCES users (id)",
        "push_installation_id UUID NOT NULL REFERENCES push_installations (id)",
        "installation_owner_epoch BIGINT NOT NULL",
        "message_preview_enabled_snapshot BOOLEAN NOT NULL",
        "claim_generation BIGINT NOT NULL DEFAULT 0",
        "CONSTRAINT uq_push_delivery_source_installation",
        "UNIQUE (source_event_id, push_installation_id)",
        "CONSTRAINT push_delivery_status_check",
        "CONSTRAINT push_delivery_claim_state_check",
        "CONSTRAINT push_delivery_terminal_state_check",
        "CREATE INDEX ix_push_delivery_due",
        "CREATE INDEX ix_push_delivery_claim_expiry",
        "CREATE INDEX ix_push_delivery_installation_epoch",
    ] {
        assert!(sql.contains(required), "migration is missing: {required}");
    }
    for forbidden in [
        "DROP TABLE",
        "DROP COLUMN",
        "raw_message_body",
        "rendered_preview",
    ] {
        assert!(
            !sql.contains(forbidden),
            "notification migration contains forbidden state or destructive DDL: {forbidden}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn notifications_push_migration_upgrades_the_exact_0006_predecessor() -> TestResult {
    let database = TestDatabase::migrated_to(6).await?;
    let mut connection = database.connection().await?;
    for table in [
        "notifications",
        "push_installations",
        "push_delivery_intents",
    ] {
        assert!(!table_exists(&mut connection, table).await?);
    }

    let migrator = sqlx::migrate::Migrator::new(std::path::Path::new("migrations")).await?;
    migrator.run_to(7, &mut connection).await?;

    for table in [
        "notifications",
        "push_installations",
        "push_delivery_intents",
    ] {
        assert!(table_exists(&mut connection, table).await?);
    }
    for constraint in [
        "notifications_type_check",
        "notifications_payload_check",
        "uq_push_installations_installation_id",
        "uq_push_installations_destination",
        "push_installations_owner_epoch_check",
        "push_installations_platform_check",
        "push_installations_provider_check",
        "push_installations_environment_check",
        "uq_push_delivery_source_installation",
        "push_delivery_status_check",
        "push_delivery_claim_state_check",
        "push_delivery_terminal_state_check",
    ] {
        assert!(
            constraint_exists(&mut connection, constraint).await?,
            "missing task-9 constraint after upgrade: {constraint}"
        );
    }
    for index in [
        "ux_notifications_user_dedup",
        "ix_notifications_user_created",
        "ix_notifications_user_unread",
        "ix_notifications_topic_cursor",
        "ix_push_installations_user",
        "ix_push_delivery_due",
        "ix_push_delivery_claim_expiry",
        "ix_push_delivery_recipient",
        "ix_push_delivery_installation_epoch",
    ] {
        assert!(
            index_exists(&mut connection, index).await?,
            "missing task-9 index after upgrade: {index}"
        );
    }
    let applied: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM _sqlx_migrations WHERE success AND version BETWEEN 1 AND 7",
    )
    .fetch_one(&mut connection)
    .await?;
    assert_eq!(applied, 7);

    connection.close().await?;
    database.dispose().await
}

#[tokio::test]
async fn a_failed_0007_upgrade_rolls_back_every_notification_and_push_relation() -> TestResult {
    let database = TestDatabase::migrated_to(6).await?;
    let fixture_dir = env::temp_dir().join(format!(
        "jamye-server-task-9-forced-migration-{}",
        Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&fixture_dir)?;
    for migration in [
        "0001_core_reliable_messaging.sql",
        "0002_auth_sessions.sql",
        "0003_invites.sql",
        "0004_chatroom_reads.sql",
        "0005_topics.sql",
        "0006_media.sql",
    ] {
        fs::copy(
            format!("migrations/{migration}"),
            fixture_dir.join(migration),
        )?;
    }
    let task_9_sql = fs::read_to_string(NOTIFICATIONS_MIGRATION)?;
    fs::write(
        fixture_dir.join("0007_forced_notifications_failure.sql"),
        format!("{task_9_sql}\n\nSELECT * FROM task_9_relation_that_must_not_exist;\n"),
    )?;

    let result: TestResult = async {
        let migrator = sqlx::migrate::Migrator::new(fixture_dir.as_path()).await?;
        let mut connection = database.connection().await?;
        let migration = migrator.run_to(7, &mut connection).await;
        assert!(
            migration.is_err(),
            "forced task-9 migration unexpectedly passed"
        );
        for table in [
            "notifications",
            "push_installations",
            "push_delivery_intents",
        ] {
            assert!(!table_exists(&mut connection, table).await?);
        }
        let applied: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM _sqlx_migrations WHERE success AND version = 7",
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

async fn constraint_exists(
    connection: &mut sqlx::PgConnection,
    constraint: &str,
) -> TestResult<bool> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = $1)",
    )
    .bind(constraint)
    .fetch_one(connection)
    .await?)
}

async fn index_exists(connection: &mut sqlx::PgConnection, index: &str) -> TestResult<bool> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS ( \
             SELECT 1 FROM pg_indexes \
             WHERE schemaname = 'public' AND indexname = $1 \
         )",
    )
    .bind(index)
    .fetch_one(connection)
    .await?)
}
