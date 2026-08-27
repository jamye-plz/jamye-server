use std::{
    env,
    error::Error,
    fs,
    future::Future,
    io,
    path::{Path, PathBuf},
};

use sqlx::{AssertSqlSafe, Connection, PgConnection, Row};
use url::Url;
use uuid::Uuid;

const CORE_MIGRATION: &str = "migrations/0001_core_reliable_messaging.sql";
const MIGRATION_POLICY: &str = "docs/adr/0003-forward-only-sqlx-migrations.md";
const DISPOSABLE_DATABASE_PREFIX: &str = "jamye_task_3a_";

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[test]
fn migration_source_is_exactly_forward_only_0001() -> TestResult {
    let migration = Path::new(CORE_MIGRATION);
    if !migration.is_file() {
        return Err(test_error(format!(
            "RED: {CORE_MIGRATION} is absent; task-3a must add the selected D1=A migration"
        )));
    }

    let mut version_one_files = fs::read_dir("migrations")?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("0001_") && name.ends_with(".sql"))
        .collect::<Vec<_>>();
    version_one_files.sort();
    assert_eq!(
        version_one_files,
        vec![String::from("0001_core_reliable_messaging.sql")]
    );

    let sql = fs::read_to_string(migration)?;
    assert!(sql.contains("-- migration: 0001_core_reliable_messaging"));
    assert!(sql.contains("-- reversibility: forward-only"));
    assert!(sql.contains(&format!("-- recovery: {MIGRATION_POLICY}")));
    assert!(!sql.contains("-- no-transaction"));

    let policy = fs::read_to_string(MIGRATION_POLICY)?;
    for required in [
        "forward-fix migration",
        "speculative down migration",
        "exact immediate-prior schema",
        "강제 오류",
        "production restore/import/cutover",
    ] {
        assert!(
            policy.contains(required),
            "migration policy is missing required rule: {required}"
        );
    }

    Ok(())
}

#[test]
fn disposable_database_identifiers_are_allowlisted_before_sql_audit() -> TestResult {
    assert_eq!(
        quoted_disposable_identifier("jamye_task_3a_0123abcdef")?,
        "\"jamye_task_3a_0123abcdef\""
    );

    for rejected in [
        "jamye_test",
        "jamye_task_3a_UPPER",
        "jamye_task_3a_safe\"; DROP DATABASE jamye_test; --",
    ] {
        assert!(quoted_disposable_identifier(rejected).is_err());
    }

    Ok(())
}

#[tokio::test]
async fn empty_database_is_the_immediate_prior_state_for_0001() -> TestResult {
    with_disposable_database(|database_url| async move {
        let mut connection = PgConnection::connect(&database_url).await?;
        assert_eq!(
            application_tables(&mut connection).await?,
            Vec::<String>::new()
        );

        let migrator = core_migrator().await?;
        migrator.run_to(1, &mut connection).await?;

        assert_eq!(
            application_tables(&mut connection).await?,
            [
                "chatrooms",
                "conversation_events",
                "groups",
                "memberships",
                "messages",
                "outbox_events",
                "users",
            ]
            .map(String::from)
            .to_vec()
        );

        let applied: i64 =
            sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success")
                .fetch_one(&mut connection)
                .await?;
        assert_eq!(applied, 1);
        assert_eq!(
            table_columns(&mut connection, "conversation_events").await?,
            [
                "conversation_id",
                "cursor",
                "event_type",
                "event_version",
                "id",
                "occurred_at",
                "payload",
            ]
            .map(String::from)
            .to_vec()
        );
        assert_eq!(
            table_columns(&mut connection, "outbox_events").await?,
            [
                "aggregate_id",
                "aggregate_type",
                "attempt_count",
                "claim_expires_at",
                "claim_generation",
                "claim_owner",
                "conversation_event_id",
                "created_at",
                "dead_lettered_at",
                "deadline_at",
                "event_type",
                "event_version",
                "id",
                "intent_type",
                "last_error_code",
                "next_attempt_at",
                "payload",
                "published_at",
                "status",
            ]
            .map(String::from)
            .to_vec()
        );
        connection.close().await?;
        Ok(())
    })
    .await
}

#[tokio::test]
async fn core_constraints_enforce_group_chatroom_and_message_invariants() -> TestResult {
    with_disposable_database(|database_url| async move {
        let mut connection = migrated_connection(&database_url).await?;

        let owner_id = Uuid::new_v4();
        let member_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let membership_id = Uuid::new_v4();
        let main_chatroom_id = Uuid::new_v4();

        sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, $2)")
            .bind(owner_id)
            .bind("owner")
            .execute(&mut connection)
            .await?;
        sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, $2)")
            .bind(member_id)
            .bind("member")
            .execute(&mut connection)
            .await?;
        sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, $2, $3)")
            .bind(group_id)
            .bind("group")
            .bind(owner_id)
            .execute(&mut connection)
            .await?;
        sqlx::query(
            "INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, 'owner')",
        )
        .bind(membership_id)
        .bind(group_id)
        .bind(owner_id)
        .execute(&mut connection)
        .await?;
        sqlx::query(
            "INSERT INTO chatrooms (id, group_id, type) VALUES ($1, $2, 'main')",
        )
        .bind(main_chatroom_id)
        .bind(group_id)
        .execute(&mut connection)
        .await?;

        assert_constraint(
            sqlx::query(
                "INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, 'member')",
            )
            .bind(Uuid::new_v4())
            .bind(group_id)
            .bind(owner_id)
            .execute(&mut connection)
            .await,
            "uq_memberships_group_user",
        )?;
        assert_constraint(
            sqlx::query(
                "INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, 'admin')",
            )
            .bind(Uuid::new_v4())
            .bind(group_id)
            .bind(member_id)
            .execute(&mut connection)
            .await,
            "memberships_role_check",
        )?;
        assert_constraint(
            sqlx::query(
                "INSERT INTO chatrooms (id, group_id, type) VALUES ($1, $2, 'main')",
            )
            .bind(Uuid::new_v4())
            .bind(group_id)
            .execute(&mut connection)
            .await,
            "ux_chatrooms_one_main_per_group",
        )?;
        assert_constraint(
            sqlx::query(
                "INSERT INTO chatrooms (id, group_id, type, topic_id) VALUES ($1, $2, 'main', $3)",
            )
            .bind(Uuid::new_v4())
            .bind(group_id)
            .bind(Uuid::new_v4())
            .execute(&mut connection)
            .await,
            "chatrooms_type_topic_check",
        )?;

        let topic_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO chatrooms (id, group_id, type, topic_id) VALUES ($1, $2, 'topic', $3)",
        )
        .bind(Uuid::new_v4())
        .bind(group_id)
        .bind(topic_id)
        .execute(&mut connection)
        .await?;
        assert_constraint(
            sqlx::query(
                "INSERT INTO chatrooms (id, group_id, type, topic_id) VALUES ($1, $2, 'topic', $3)",
            )
            .bind(Uuid::new_v4())
            .bind(group_id)
            .bind(topic_id)
            .execute(&mut connection)
            .await,
            "ux_chatrooms_one_topic_per_topic",
        )?;

        let client_msg_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO messages (id, chatroom_id, sender_id, client_msg_id, body, type) \
             VALUES ($1, $2, $3, $4, $5, 'user')",
        )
        .bind(Uuid::new_v4())
        .bind(main_chatroom_id)
        .bind(owner_id)
        .bind(client_msg_id)
        .bind("hello")
        .execute(&mut connection)
        .await?;
        assert_constraint(
            sqlx::query(
                "INSERT INTO messages (id, chatroom_id, sender_id, client_msg_id, body, type) \
                 VALUES ($1, $2, $3, $4, $5, 'user')",
            )
            .bind(Uuid::new_v4())
            .bind(main_chatroom_id)
            .bind(owner_id)
            .bind(client_msg_id)
            .bind("duplicate")
            .execute(&mut connection)
            .await,
            "ux_messages_sender_client_msg_id",
        )?;
        sqlx::query(
            "INSERT INTO messages (id, chatroom_id, sender_id, client_msg_id, body, type) \
             VALUES ($1, $2, $3, $4, $5, 'user')",
        )
        .bind(Uuid::new_v4())
        .bind(main_chatroom_id)
        .bind(member_id)
        .bind(client_msg_id)
        .bind("same key, different sender")
        .execute(&mut connection)
        .await?;
        assert_constraint(
            sqlx::query(
                "INSERT INTO messages (id, chatroom_id, body, type) VALUES ($1, $2, $3, 'user')",
            )
            .bind(Uuid::new_v4())
            .bind(main_chatroom_id)
            .bind("missing client identity")
            .execute(&mut connection)
            .await,
            "messages_identity_check",
        )?;

        for body in ["system one", "system two"] {
            sqlx::query(
                "INSERT INTO messages (id, chatroom_id, body, type) VALUES ($1, $2, $3, 'system')",
            )
            .bind(Uuid::new_v4())
            .bind(main_chatroom_id)
            .bind(body)
            .execute(&mut connection)
            .await?;
        }

        connection.close().await?;
        Ok(())
    })
    .await
}

#[tokio::test]
async fn cursors_are_server_generated_and_outbox_claim_state_is_guarded() -> TestResult {
    with_disposable_database(|database_url| async move {
        let mut connection = migrated_connection(&database_url).await?;
        let conversation_id = Uuid::new_v4();

        let first_cursor: i64 = sqlx::query_scalar(
            "INSERT INTO conversation_events \
             (id, conversation_id, event_type, event_version, payload) \
             VALUES ($1, $2, 'message.created', 1, '{}'::jsonb) RETURNING cursor",
        )
        .bind(Uuid::new_v4())
        .bind(conversation_id)
        .fetch_one(&mut connection)
        .await?;
        let event_id = Uuid::new_v4();
        let second_cursor: i64 = sqlx::query_scalar(
            "INSERT INTO conversation_events \
             (id, conversation_id, event_type, event_version, payload) \
             VALUES ($1, $2, 'message.created', 1, '{}'::jsonb) RETURNING cursor",
        )
        .bind(event_id)
        .bind(conversation_id)
        .fetch_one(&mut connection)
        .await?;
        assert!(second_cursor > first_cursor);

        assert_database_error(
            sqlx::query(
                "INSERT INTO conversation_events \
                 (id, cursor, conversation_id, event_type, event_version, payload) \
                 VALUES ($1, 999, $2, 'message.created', 1, '{}'::jsonb)",
            )
            .bind(Uuid::new_v4())
            .bind(conversation_id)
            .execute(&mut connection)
            .await,
            "client-supplied conversation cursor was accepted",
        )?;

        let row = sqlx::query(
            "INSERT INTO outbox_events \
             (id, intent_type, event_type, event_version, aggregate_type, aggregate_id, \
              conversation_event_id, payload) \
             VALUES ($1, 'conversation', 'message.created', 1, 'conversation', $2, $3, '{}'::jsonb) \
             RETURNING status, claim_generation, attempt_count",
        )
        .bind(Uuid::new_v4())
        .bind(conversation_id)
        .bind(event_id)
        .fetch_one(&mut connection)
        .await?;
        assert_eq!(row.try_get::<String, _>("status")?, "pending");
        assert_eq!(row.try_get::<i64, _>("claim_generation")?, 0);
        assert_eq!(row.try_get::<i32, _>("attempt_count")?, 0);

        assert_constraint(
            sqlx::query(
                "INSERT INTO outbox_events \
                 (id, intent_type, event_type, event_version, aggregate_type, aggregate_id, payload, status) \
                 VALUES ($1, 'control', 'membership.revoked', 1, 'membership', $2, '{}'::jsonb, 'claimed')",
            )
            .bind(Uuid::new_v4())
            .bind(Uuid::new_v4())
            .execute(&mut connection)
            .await,
            "outbox_claim_state_check",
        )?;
        assert_constraint(
            sqlx::query(
                "INSERT INTO outbox_events \
                 (id, intent_type, event_type, event_version, aggregate_type, aggregate_id, \
                  conversation_event_id, payload) \
                 VALUES ($1, 'control', 'membership.revoked', 1, 'membership', $2, $3, '{}'::jsonb)",
            )
            .bind(Uuid::new_v4())
            .bind(Uuid::new_v4())
            .bind(event_id)
            .execute(&mut connection)
            .await,
            "outbox_intent_shape_check",
        )?;
        sqlx::query(
            "INSERT INTO outbox_events \
             (id, intent_type, event_type, event_version, aggregate_type, aggregate_id, payload) \
             VALUES ($1, 'control', 'membership.revoked', 1, 'membership', $2, '{}'::jsonb)",
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .execute(&mut connection)
        .await?;

        connection.close().await?;
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_failed_sqlx_migration_leaves_no_partial_core_schema() -> TestResult {
    with_disposable_database(|database_url| async move {
        let migration_sql = fs::read_to_string(CORE_MIGRATION)?;
        let fixture_dir = forced_failure_fixture_dir();
        fs::create_dir_all(&fixture_dir)?;
        let fixture_path = fixture_dir.join("0001_forced_failure.sql");
        fs::write(
            &fixture_path,
            format!("{migration_sql}\n\nSELECT * FROM task_3a_relation_that_must_not_exist;\n"),
        )?;

        let migration_result: TestResult = async {
            let migrator = sqlx::migrate::Migrator::new(fixture_dir.as_path()).await?;
            let mut connection = PgConnection::connect(&database_url).await?;
            let result = migrator.run(&mut connection).await;
            assert!(result.is_err(), "forced migration unexpectedly succeeded");
            assert_eq!(
                application_tables(&mut connection).await?,
                Vec::<String>::new()
            );
            connection.close().await?;
            Ok(())
        }
        .await;

        let cleanup_result = fs::remove_dir_all(&fixture_dir);
        if let Err(error) = cleanup_result {
            return Err(test_error(format!(
                "failed to remove forced-migration fixture {}: {error}",
                fixture_dir.display()
            )));
        }

        migration_result
    })
    .await
}

async fn core_migrator() -> TestResult<sqlx::migrate::Migrator> {
    let migrator = sqlx::migrate::Migrator::new(Path::new("migrations")).await?;
    let version_one = migrator
        .iter()
        .find(|migration| migration.version == 1)
        .ok_or_else(|| test_error("SQLx migration source does not contain version 1"))?;
    assert!(!version_one.no_tx);
    Ok(migrator)
}

async fn migrated_connection(database_url: &str) -> TestResult<PgConnection> {
    let mut connection = PgConnection::connect(database_url).await?;
    core_migrator().await?.run_to(1, &mut connection).await?;
    Ok(connection)
}

async fn application_tables(connection: &mut PgConnection) -> TestResult<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT tablename::text \
         FROM pg_catalog.pg_tables \
         WHERE schemaname = 'public' AND tablename <> '_sqlx_migrations' \
         ORDER BY tablename",
    )
    .fetch_all(connection)
    .await?)
}

async fn table_columns(
    connection: &mut PgConnection,
    table_name: &'static str,
) -> TestResult<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT column_name::text \
         FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = $1 \
         ORDER BY column_name",
    )
    .bind(table_name)
    .fetch_all(connection)
    .await?)
}

async fn with_disposable_database<F, Fut>(test: F) -> TestResult
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = TestResult>,
{
    let base_url = env::var("DATABASE_URL")
        .map_err(|_| test_error("DATABASE_URL is required for task-3a GREEN tests"))?;
    let mut parsed = validate_disposable_database_url(&base_url)?;
    let database_name = format!("{DISPOSABLE_DATABASE_PREFIX}{}", Uuid::new_v4().simple());
    let database_identifier = quoted_disposable_identifier(&database_name)?;

    let mut admin = PgConnection::connect(&base_url).await?;
    sqlx::query(AssertSqlSafe(format!(
        "CREATE DATABASE {database_identifier}"
    )))
    .execute(&mut admin)
    .await?;

    parsed.set_path(&format!("/{database_name}"));
    let test_result = test(parsed.to_string()).await;
    let drop_result = sqlx::query(AssertSqlSafe(format!(
        "DROP DATABASE {database_identifier} WITH (FORCE)"
    )))
    .execute(&mut admin)
    .await;

    match (test_result, drop_result) {
        (Ok(()), Ok(_)) => Ok(()),
        (Err(test_error), Ok(_)) => Err(test_error),
        (Ok(()), Err(drop_error)) => Err(Box::new(drop_error)),
        (Err(test_failure), Err(drop_error)) => Err(test_error(format!(
            "test failed: {test_failure}; cleanup also failed for {database_name}: {drop_error}"
        ))),
    }
}

fn validate_disposable_database_url(database_url: &str) -> TestResult<Url> {
    let environment = env::var("JAMYE_ENVIRONMENT")
        .map_err(|_| test_error("JAMYE_ENVIRONMENT is required for task-3a GREEN tests"))?;
    if environment != "test" {
        return Err(test_error(
            "task-3a database tests refuse to run unless JAMYE_ENVIRONMENT=test",
        ));
    }

    let parsed = Url::parse(database_url)?;
    if !matches!(parsed.scheme(), "postgres" | "postgresql") {
        return Err(test_error("task-3a requires a PostgreSQL DATABASE_URL"));
    }
    if !matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "::1")) {
        return Err(test_error(
            "task-3a database tests only accept a loopback PostgreSQL host",
        ));
    }
    if parsed.path() != "/jamye_test" {
        return Err(test_error(
            "task-3a database tests only accept the disposable jamye_test database",
        ));
    }

    Ok(parsed)
}

fn quoted_disposable_identifier(identifier: &str) -> TestResult<String> {
    let valid = identifier.starts_with(DISPOSABLE_DATABASE_PREFIX)
        && identifier.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        });
    if !valid {
        return Err(test_error("refused unsafe disposable database identifier"));
    }
    Ok(format!("\"{identifier}\""))
}

fn forced_failure_fixture_dir() -> PathBuf {
    env::temp_dir().join(format!(
        "jamye-server-task-3a-forced-migration-{}",
        Uuid::new_v4().simple()
    ))
}

fn assert_constraint(
    result: Result<sqlx::postgres::PgQueryResult, sqlx::Error>,
    expected_constraint: &str,
) -> TestResult {
    match result {
        Ok(_) => Err(test_error(format!(
            "expected constraint {expected_constraint} to reject the write"
        ))),
        Err(sqlx::Error::Database(database_error))
            if database_error.constraint() == Some(expected_constraint) =>
        {
            Ok(())
        }
        Err(error) => Err(test_error(format!(
            "expected constraint {expected_constraint}, received {error}"
        ))),
    }
}

fn assert_database_error(
    result: Result<sqlx::postgres::PgQueryResult, sqlx::Error>,
    success_message: &str,
) -> TestResult {
    match result {
        Ok(_) => Err(test_error(success_message)),
        Err(sqlx::Error::Database(_)) => Ok(()),
        Err(error) => Err(Box::new(error)),
    }
}

fn test_error(message: impl Into<String>) -> Box<dyn Error + Send + Sync> {
    Box::new(io::Error::other(message.into()))
}
