//! Migration 0020: additive forward-only schema, 0019-to-0020 upgrade and forced rollback.

use std::{env, fs, path::Path};

use sqlx::{Connection, PgConnection};
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

const MIGRATION: &str = "migrations/0020_ugc_moderation.sql";

#[test]
fn migration_0020_is_forward_only_and_has_no_terms_or_audit_table() -> TestResult {
    let sql = fs::read_to_string(MIGRATION)?;
    for required in [
        "-- migration: 0020_ugc_moderation",
        "-- prerequisite: 0019_user_avatar_uploads.sql",
        "-- reversibility: forward-only",
        "-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md",
        "-- rationale:",
        "-- lock impact:",
        "CREATE TABLE user_blocks",
        "CREATE TABLE reports",
        "ADD COLUMN suspended_at TIMESTAMPTZ",
        "ADD COLUMN suspension_reason TEXT",
        "CHECK (blocker_id <> blocked_id)",
        "status IN ('open', 'actioned', 'dismissed')",
    ] {
        assert!(
            sql.contains(required),
            "0020 migration is missing: {required}"
        );
    }
    for forbidden in [
        "CREATE TABLE terms",
        "terms_acceptance",
        "moderation_actions",
        "audit",
        "DROP TABLE",
        "DROP COLUMN",
        "DROP CONSTRAINT",
        "TRUNCATE",
        "INSERT INTO",
        "-- no-transaction",
    ] {
        assert!(
            !sql.contains(forbidden),
            "0020 migration is not additive: {forbidden}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn migration_0020_indexes_the_report_target_columns_used_by_purge_and_actioning() -> TestResult
{
    let database = TestDatabase::migrated().await?;
    let mut connection = database.connection().await?;
    let result: TestResult = async {
        // Account purge reassigns reports by target user, and the operator actions the open
        // reports of a message: both lookups must not scan the table.
        for (index, column) in [
            ("ix_reports_target_message", "target_message_id"),
            ("ix_reports_target_user", "target_user_id"),
        ] {
            let definition = sqlx::query_scalar::<_, String>(
                "SELECT indexdef FROM pg_indexes \
                 WHERE schemaname = current_schema() \
                   AND tablename = 'reports' AND indexname = $1",
            )
            .bind(index)
            .fetch_optional(&mut connection)
            .await?
            .ok_or_else(|| format!("index {index} is missing"))?;
            assert!(
                definition.contains(&format!("({column})")),
                "{index} does not index {column}: {definition}"
            );
        }
        Ok(())
    }
    .await;
    connection.close().await?;
    database.dispose().await?;
    result
}

#[tokio::test]
async fn migration_0020_upgrades_0019_and_enforces_the_moderation_rules() -> TestResult {
    let database = TestDatabase::migrated_to(19).await?;
    let mut connection = database.connection().await?;
    let result: TestResult = async {
        assert!(!table_exists(&mut connection, "reports").await?);
        assert!(!table_exists(&mut connection, "user_blocks").await?);
        assert!(!column_exists(&mut connection, "users", "suspended_at").await?);
        assert!(!column_exists(&mut connection, "push_delivery_intents", "report_id").await?);
        let (user_id, group_id, message_id) = seed_message(&mut connection).await?;

        let migrator = sqlx::migrate::Migrator::new(Path::new("migrations")).await?;
        migrator.run_to(20, &mut connection).await?;
        let applied: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM _sqlx_migrations WHERE success AND version BETWEEN 1 AND 20",
        )
        .fetch_one(&mut connection)
        .await?;
        assert_eq!(applied, 20);

        assert_eq!(
            columns(&mut connection, "user_blocks").await?,
            ["blocked_id", "blocker_id", "created_at"]
        );
        assert_eq!(
            columns(&mut connection, "reports").await?,
            [
                "created_at",
                "handled_at",
                "id",
                "message_snapshot",
                "reason",
                "reporter_id",
                "status",
                "target_group_id",
                "target_message_id",
                "target_type",
                "target_user_id",
            ]
        );
        assert!(column_exists(&mut connection, "users", "suspended_at").await?);
        assert!(column_exists(&mut connection, "users", "suspension_reason").await?);
        let nullable_suspension = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name = 'users' \
               AND column_name IN ('suspended_at', 'suspension_reason') \
               AND is_nullable = 'YES'",
        )
        .fetch_one(&mut connection)
        .await?;
        assert_eq!(nullable_suspension, 2);
        // Existing accounts are untouched: nobody is suspended after the upgrade.
        let suspended: i64 =
            sqlx::query_scalar("SELECT count(*) FROM users WHERE suspended_at IS NOT NULL")
                .fetch_one(&mut connection)
                .await?;
        assert_eq!(suspended, 0);
        for forbidden in ["terms", "terms_acceptances", "moderation_actions"] {
            assert!(!table_exists(&mut connection, forbidden).await?);
        }

        // Blocks: one row per direction, never self.
        let other = insert_user(&mut connection).await?;
        sqlx::query("INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1, $2)")
            .bind(user_id)
            .bind(other)
            .execute(&mut connection)
            .await?;
        assert!(
            sqlx::query("INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1, $2)")
                .bind(user_id)
                .bind(other)
                .execute(&mut connection)
                .await
                .is_err(),
            "duplicate block was accepted"
        );
        assert!(
            sqlx::query("INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1, $1)")
                .bind(user_id)
                .execute(&mut connection)
                .await
                .is_err(),
            "self block was accepted"
        );
        sqlx::query("INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1, $2)")
            .bind(other)
            .bind(user_id)
            .execute(&mut connection)
            .await?;

        // Reports: exactly one target, bounded vocabularies, handled state.
        let snapshot = serde_json::json!({"text": "x", "content_type": "text", "media": []});
        let message_report = insert_report(
            &mut connection,
            "message",
            Some(message_id),
            None,
            group_id,
            user_id,
            "spam",
            Some(&snapshot),
            "open",
            false,
        )
        .await;
        assert!(message_report.is_ok(), "valid message report was rejected");
        assert!(
            insert_report(
                &mut connection,
                "user",
                None,
                Some(other),
                group_id,
                user_id,
                "hate",
                None,
                "open",
                false,
            )
            .await
            .is_ok(),
            "valid user report was rejected"
        );
        for (label, rejected) in [
            (
                "both targets",
                insert_report(
                    &mut connection,
                    "message",
                    Some(message_id),
                    Some(other),
                    group_id,
                    user_id,
                    "spam",
                    Some(&snapshot),
                    "open",
                    false,
                )
                .await,
            ),
            (
                "no target",
                insert_report(
                    &mut connection,
                    "user",
                    None,
                    None,
                    group_id,
                    user_id,
                    "spam",
                    None,
                    "open",
                    false,
                )
                .await,
            ),
            (
                "message without snapshot",
                insert_report(
                    &mut connection,
                    "message",
                    Some(message_id),
                    None,
                    group_id,
                    user_id,
                    "spam",
                    None,
                    "open",
                    false,
                )
                .await,
            ),
            (
                "user with snapshot",
                insert_report(
                    &mut connection,
                    "user",
                    None,
                    Some(other),
                    group_id,
                    user_id,
                    "spam",
                    Some(&snapshot),
                    "open",
                    false,
                )
                .await,
            ),
            (
                "unknown reason",
                insert_report(
                    &mut connection,
                    "user",
                    None,
                    Some(other),
                    group_id,
                    user_id,
                    "rude",
                    None,
                    "open",
                    false,
                )
                .await,
            ),
            (
                "unknown status",
                insert_report(
                    &mut connection,
                    "user",
                    None,
                    Some(other),
                    group_id,
                    user_id,
                    "spam",
                    None,
                    "closed",
                    false,
                )
                .await,
            ),
            (
                "actioned without handled_at",
                insert_report(
                    &mut connection,
                    "user",
                    None,
                    Some(other),
                    group_id,
                    user_id,
                    "spam",
                    None,
                    "actioned",
                    false,
                )
                .await,
            ),
            (
                "open with handled_at",
                insert_report(
                    &mut connection,
                    "user",
                    None,
                    Some(other),
                    group_id,
                    user_id,
                    "spam",
                    None,
                    "open",
                    true,
                )
                .await,
            ),
        ] {
            assert!(rejected.is_err(), "invalid report accepted: {label}");
        }
        assert!(
            insert_report(
                &mut connection,
                "user",
                None,
                Some(other),
                group_id,
                user_id,
                "other",
                None,
                "dismissed",
                true,
            )
            .await
            .is_ok()
        );

        // Suspension columns: a reason needs a timestamp and stays bounded.
        assert!(
            sqlx::query("UPDATE users SET suspension_reason = 'x' WHERE id = $1")
                .bind(user_id)
                .execute(&mut connection)
                .await
                .is_err(),
            "a reason without suspended_at was accepted"
        );
        sqlx::query(
            "UPDATE users SET suspended_at = clock_timestamp(), suspension_reason = 'x' WHERE id = $1",
        )
        .bind(user_id)
        .execute(&mut connection)
        .await?;
        assert!(
            sqlx::query("UPDATE users SET suspension_reason = $2 WHERE id = $1")
                .bind(user_id)
                .bind("x".repeat(501))
                .execute(&mut connection)
                .await
                .is_err(),
            "an oversized reason was accepted"
        );

        // The push occurrence table accepts exactly the two shapes.
        let installation = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO push_installations (id, user_id, installation_id, platform, token, environment) \
             VALUES ($1, $2, $3, 'ios', $4, 'development')",
        )
        .bind(installation)
        .bind(user_id)
        .bind(format!("installation-{installation}"))
        .bind(format!("token-{installation}"))
        .execute(&mut connection)
        .await?;
        let report_id: Uuid = sqlx::query_scalar("SELECT id FROM reports LIMIT 1")
            .fetch_one(&mut connection)
            .await?;
        assert!(
            insert_alert(&mut connection, None, user_id, installation)
                .await
                .is_err(),
            "an intent without any subject was accepted"
        );
        insert_alert(&mut connection, Some(report_id), user_id, installation).await?;
        assert!(
            insert_alert(&mut connection, Some(report_id), user_id, installation)
                .await
                .is_err(),
            "a second alert for the same report and installation was accepted"
        );
        Ok(())
    }
    .await;

    connection.close().await?;
    database.dispose().await?;
    result
}

#[tokio::test]
async fn migration_0020_keeps_existing_push_occurrences_in_their_conversation_shape() -> TestResult
{
    let database = TestDatabase::migrated_to(19).await?;
    let mut connection = database.connection().await?;
    let result: TestResult = async {
        let (user_id, _group_id, message_id) = seed_message(&mut connection).await?;
        let chatroom_id: Uuid = sqlx::query_scalar("SELECT chatroom_id FROM messages WHERE id = $1")
            .bind(message_id)
            .fetch_one(&mut connection)
            .await?;
        let event_id = Uuid::new_v4();
        let cursor: i64 = sqlx::query_scalar(
            "INSERT INTO conversation_events \
                 (id, conversation_id, event_type, event_version, payload) \
             VALUES ($1, $2, 'message.created', 1, '{}'::jsonb) \
             RETURNING cursor",
        )
        .bind(event_id)
        .bind(chatroom_id)
        .fetch_one(&mut connection)
        .await?;
        let notification_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO notifications \
                 (id, user_id, conversation_id, source_cursor, type, payload, dedup_key) \
             VALUES ($1, $2, $3, $4, 'chat_unread', '{}'::jsonb, $5)",
        )
        .bind(notification_id)
        .bind(user_id)
        .bind(chatroom_id)
        .bind(cursor)
        .bind(format!("chat_unread:conversation:{chatroom_id}"))
        .execute(&mut connection)
        .await?;
        let installation = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO push_installations (id, user_id, installation_id, platform, token, environment) \
             VALUES ($1, $2, $3, 'ios', $4, 'development')",
        )
        .bind(installation)
        .bind(user_id)
        .bind(format!("installation-{installation}"))
        .bind(format!("token-{installation}"))
        .execute(&mut connection)
        .await?;
        let intent_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO push_delivery_intents \
                 (id, notification_id, source_event_id, source_message_id, recipient_user_id, \
                  push_installation_id, installation_owner_epoch, \
                  message_preview_enabled_snapshot, payload) \
             VALUES ($1, $2, $3, $4, $5, $6, 1, false, '{}'::jsonb)",
        )
        .bind(intent_id)
        .bind(notification_id)
        .bind(event_id)
        .bind(message_id)
        .bind(user_id)
        .bind(installation)
        .execute(&mut connection)
        .await?;

        let migrator = sqlx::migrate::Migrator::new(Path::new("migrations")).await?;
        migrator.run_to(20, &mut connection).await?;

        // The pre-existing occurrence survives untouched and keeps both references.
        let kept = sqlx::query_as::<_, (i64, bool, bool, bool, String)>(
            "SELECT count(*), bool_and(notification_id = $2), bool_and(source_event_id = $3), \
                    bool_and(report_id IS NULL), min(status) \
             FROM push_delivery_intents WHERE id = $1",
        )
        .bind(intent_id)
        .bind(notification_id)
        .bind(event_id)
        .fetch_one(&mut connection)
        .await?;
        assert_eq!(kept, (1, true, true, true, "pending".to_owned()));

        // New rows still need the full conversation shape unless they belong to a report.
        let half_filled = sqlx::query(
            "INSERT INTO push_delivery_intents \
                 (id, notification_id, recipient_user_id, push_installation_id, \
                  installation_owner_epoch, message_preview_enabled_snapshot, payload) \
             VALUES ($1, $2, $3, $4, 1, false, '{}'::jsonb)",
        )
        .bind(Uuid::new_v4())
        .bind(notification_id)
        .bind(user_id)
        .bind(installation)
        .execute(&mut connection)
        .await;
        assert!(
            half_filled.is_err(),
            "a notification occurrence without a conversation event was accepted"
        );
        // The existing per-event, per-installation uniqueness still holds.
        let duplicate = sqlx::query(
            "INSERT INTO push_delivery_intents \
                 (id, notification_id, source_event_id, recipient_user_id, push_installation_id, \
                  installation_owner_epoch, message_preview_enabled_snapshot, payload) \
             VALUES ($1, $2, $3, $4, $5, 1, false, '{}'::jsonb)",
        )
        .bind(Uuid::new_v4())
        .bind(notification_id)
        .bind(event_id)
        .bind(user_id)
        .bind(installation)
        .execute(&mut connection)
        .await;
        assert!(duplicate.is_err(), "a duplicate occurrence was accepted");
        Ok(())
    }
    .await;

    connection.close().await?;
    database.dispose().await?;
    result
}

#[tokio::test]
async fn a_failed_0020_upgrade_rolls_back_every_moderation_change() -> TestResult {
    let database = TestDatabase::migrated_to(19).await?;
    let fixture_dir = env::temp_dir().join(format!(
        "jamye-server-task-20b-forced-migration-{}",
        Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&fixture_dir)?;
    for entry in fs::read_dir("migrations")? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| std::io::Error::other("migration filename is not UTF-8"))?;
        if name.ends_with(".sql") && name < "0020_ugc_moderation.sql" {
            fs::copy(entry.path(), fixture_dir.join(name))?;
        }
    }
    let moderation_sql = fs::read_to_string(MIGRATION)?;
    fs::write(
        fixture_dir.join("0020_ugc_moderation.sql"),
        format!("{moderation_sql}\n\nSELECT * FROM task_20b_relation_that_must_not_exist;\n"),
    )?;

    let result: TestResult = async {
        let migrator = sqlx::migrate::Migrator::new(fixture_dir.as_path()).await?;
        let mut connection = database.connection().await?;
        assert!(migrator.run_to(20, &mut connection).await.is_err());
        for table in ["reports", "user_blocks"] {
            assert!(
                !table_exists(&mut connection, table).await?,
                "failed 0020 left {table} behind"
            );
        }
        assert!(!column_exists(&mut connection, "users", "suspended_at").await?);
        assert!(!column_exists(&mut connection, "users", "suspension_reason").await?);
        assert!(!column_exists(&mut connection, "push_delivery_intents", "report_id").await?);
        let notification_not_null: String = sqlx::query_scalar(
            "SELECT is_nullable FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name = 'push_delivery_intents' \
               AND column_name = 'notification_id'",
        )
        .fetch_one(&mut connection)
        .await?;
        assert_eq!(
            notification_not_null, "NO",
            "failed 0020 relaxed an existing column"
        );
        let applied: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM _sqlx_migrations WHERE success AND version = 20",
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

async fn columns(connection: &mut PgConnection, table: &str) -> TestResult<Vec<String>> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT column_name FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = $1 ORDER BY column_name",
    )
    .bind(table)
    .fetch_all(connection)
    .await?)
}

async fn insert_user(connection: &mut PgConnection) -> TestResult<Uuid> {
    let user_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, 'moderation migration')")
        .bind(user_id)
        .execute(connection)
        .await?;
    Ok(user_id)
}

/// A user, a group with its main chatroom and one message, all created on the 0019 schema.
async fn seed_message(connection: &mut PgConnection) -> TestResult<(Uuid, Uuid, Uuid)> {
    let user_id = insert_user(connection).await?;
    let group_id = Uuid::new_v4();
    let chatroom_id = Uuid::new_v4();
    let message_id = Uuid::new_v4();
    sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, 'migration group', $2)")
        .bind(group_id)
        .bind(user_id)
        .execute(&mut *connection)
        .await?;
    sqlx::query("INSERT INTO chatrooms (id, group_id, type) VALUES ($1, $2, 'main')")
        .bind(chatroom_id)
        .bind(group_id)
        .execute(&mut *connection)
        .await?;
    sqlx::query(
        "INSERT INTO messages (id, chatroom_id, sender_id, client_msg_id, body, type) \
         VALUES ($1, $2, $3, $4, 'hello', 'user')",
    )
    .bind(message_id)
    .bind(chatroom_id)
    .bind(user_id)
    .bind(Uuid::new_v4())
    .execute(&mut *connection)
    .await?;
    Ok((user_id, group_id, message_id))
}

#[allow(
    clippy::too_many_arguments,
    reason = "the helper mirrors the reports columns so each case states its full row"
)]
async fn insert_report(
    connection: &mut PgConnection,
    target_type: &str,
    message_id: Option<Uuid>,
    target_user_id: Option<Uuid>,
    group_id: Uuid,
    reporter_id: Uuid,
    reason: &str,
    snapshot: Option<&serde_json::Value>,
    status: &str,
    handled: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO reports \
             (id, reporter_id, target_type, target_message_id, target_user_id, target_group_id, \
              reason, message_snapshot, status, handled_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, \
                 CASE WHEN $10 THEN clock_timestamp() ELSE NULL END)",
    )
    .bind(Uuid::new_v4())
    .bind(reporter_id)
    .bind(target_type)
    .bind(message_id)
    .bind(target_user_id)
    .bind(group_id)
    .bind(reason)
    .bind(snapshot)
    .bind(status)
    .bind(handled)
    .execute(connection)
    .await
    .map(|_| ())
}

async fn insert_alert(
    connection: &mut PgConnection,
    report_id: Option<Uuid>,
    user_id: Uuid,
    installation: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO push_delivery_intents \
             (id, report_id, recipient_user_id, push_installation_id, \
              installation_owner_epoch, message_preview_enabled_snapshot, payload) \
         VALUES ($1, $2, $3, $4, 1, false, '{}'::jsonb)",
    )
    .bind(Uuid::new_v4())
    .bind(report_id)
    .bind(user_id)
    .bind(installation)
    .execute(connection)
    .await
    .map(|_| ())
}
