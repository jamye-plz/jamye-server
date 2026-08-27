use std::{env, fs, io};

use sqlx::Connection;
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

const MEDIA_MIGRATION: &str = "migrations/0006_media.sql";

#[test]
fn media_migration_is_forward_only_and_owns_one_time_binding_constraints() -> TestResult {
    let sql = fs::read_to_string(MEDIA_MIGRATION).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            io::Error::other(format!(
                "RED: {MEDIA_MIGRATION} is absent; task-8 must add authoritative upload and ordered one-time media binding state"
            ))
        } else {
            error
        }
    })?;

    for required in [
        "-- migration: 0006_media",
        "-- prerequisite: 0005_topics.sql",
        "-- reversibility: forward-only",
        "CREATE TABLE media_uploads",
        "CREATE TABLE message_media",
        "ADD COLUMN media_upload_id UUID NOT NULL",
        "CONSTRAINT media_uploads_consumer_shape_check",
        "CONSTRAINT uq_media_uploads_bound_message_pair UNIQUE (id, bound_message_id)",
        "CONSTRAINT uq_media_uploads_bound_topic_pair UNIQUE (id, bound_topic_media_id)",
        "CONSTRAINT uq_message_media_upload UNIQUE (media_upload_id)",
        "CONSTRAINT uq_message_media_message_position UNIQUE (message_id, position)",
        "CONSTRAINT message_media_position_check CHECK (position BETWEEN 0 AND 3)",
        "filename VARCHAR(255)",
        "filename IS NULL OR length(filename) <= 255",
        "CONSTRAINT uq_topic_media_upload UNIQUE (media_upload_id)",
        "CONSTRAINT fk_message_media_bound_upload",
        "CONSTRAINT fk_topic_media_bound_upload",
        "DEFERRABLE INITIALLY DEFERRED",
    ] {
        assert!(sql.contains(required), "migration is missing: {required}");
    }
    for forbidden in ["DROP TABLE", "DROP COLUMN"] {
        assert!(
            !sql.contains(forbidden),
            "forward-only media migration contains destructive DDL: {forbidden}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn media_migration_upgrades_the_exact_0005_predecessor() -> TestResult {
    let database = TestDatabase::migrated_to(5).await?;
    let mut connection = database.connection().await?;
    assert!(!table_exists(&mut connection, "media_uploads").await?);
    assert!(!table_exists(&mut connection, "message_media").await?);
    assert!(!column_exists(&mut connection, "topic_media", "media_upload_id").await?);

    let migrator = sqlx::migrate::Migrator::new(std::path::Path::new("migrations")).await?;
    migrator.run_to(6, &mut connection).await?;

    for table in ["media_uploads", "message_media"] {
        assert!(table_exists(&mut connection, table).await?);
    }
    assert!(column_exists(&mut connection, "topic_media", "media_upload_id").await?);
    for constraint in [
        "media_uploads_consumer_shape_check",
        "uq_message_media_upload",
        "uq_message_media_message_position",
        "uq_topic_media_upload",
        "fk_message_media_bound_upload",
        "fk_topic_media_bound_upload",
    ] {
        assert!(
            constraint_exists(&mut connection, constraint).await?,
            "missing media constraint after upgrade: {constraint}"
        );
    }
    let applied: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM _sqlx_migrations WHERE success AND version BETWEEN 1 AND 6",
    )
    .fetch_one(&mut connection)
    .await?;
    assert_eq!(applied, 6);

    connection.close().await?;
    database.dispose().await
}

#[tokio::test]
async fn media_schema_allows_four_ordered_attachments_and_rejects_cross_consumption() -> TestResult
{
    let database = TestDatabase::migrated().await?;
    let mut connection = database.connection().await?;
    let user_id = Uuid::new_v4();
    let group_id = Uuid::new_v4();
    let topic_id = Uuid::new_v4();
    let chatroom_id = Uuid::new_v4();
    let message_id = Uuid::new_v4();

    sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, 'media owner')")
        .bind(user_id)
        .execute(&mut connection)
        .await?;
    sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, 'media group', $2)")
        .bind(group_id)
        .bind(user_id)
        .execute(&mut connection)
        .await?;
    sqlx::query(
        "INSERT INTO topics (id, group_id, author_id, idempotency_key, request_fingerprint, title) \
         VALUES ($1, $2, $3, $4, $5, 'media topic')",
    )
    .bind(topic_id)
    .bind(group_id)
    .bind(user_id)
    .bind(Uuid::new_v4())
    .bind("a".repeat(64))
    .execute(&mut connection)
    .await?;
    sqlx::query("INSERT INTO chatrooms (id, group_id, type) VALUES ($1, $2, 'main')")
        .bind(chatroom_id)
        .bind(group_id)
        .execute(&mut connection)
        .await?;
    sqlx::query(
        "INSERT INTO messages (id, chatroom_id, sender_id, client_msg_id, body) \
         VALUES ($1, $2, $3, $4, 'four attachments')",
    )
    .bind(message_id)
    .bind(chatroom_id)
    .bind(user_id)
    .bind(Uuid::new_v4())
    .execute(&mut connection)
    .await?;

    let mut upload_ids = Vec::new();
    for position in 0..4 {
        let upload_id = Uuid::new_v4();
        insert_bound_chat_attachment(
            &mut connection,
            upload_id,
            user_id,
            chatroom_id,
            message_id,
            position,
        )
        .await?;
        upload_ids.push(upload_id);
    }

    let attachment_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM message_media WHERE message_id = $1")
            .bind(message_id)
            .fetch_one(&mut connection)
            .await?;
    assert_eq!(attachment_count, 4);

    let fifth_upload_id = Uuid::new_v4();
    let mut fifth = connection.begin().await?;
    insert_bound_chat_upload(
        &mut fifth,
        fifth_upload_id,
        user_id,
        chatroom_id,
        message_id,
        "chat/fifth",
    )
    .await?;
    let fifth_result = sqlx::query(
        "INSERT INTO message_media \
         (id, message_id, media_upload_id, type, object_key, byte_size, position) \
         VALUES ($1, $2, $3, 'image/jpeg', 'chat/fifth', 42, 4)",
    )
    .bind(Uuid::new_v4())
    .bind(message_id)
    .bind(fifth_upload_id)
    .execute(&mut *fifth)
    .await;
    assert!(
        fifth_result.is_err(),
        "a fifth attachment unexpectedly passed"
    );
    fifth.rollback().await?;

    let mut cross_consumer = connection.begin().await?;
    sqlx::query(
        "INSERT INTO topic_media \
         (id, topic_id, media_upload_id, type, object_key, width, height, byte_size) \
         VALUES ($1, $2, $3, 'image/jpeg', 'topic/cross-consumer', 1, 1, 42)",
    )
    .bind(Uuid::new_v4())
    .bind(topic_id)
    .bind(upload_ids[0])
    .execute(&mut *cross_consumer)
    .await?;
    let cross_result = cross_consumer.commit().await;
    assert!(
        cross_result.is_err(),
        "one upload unexpectedly acquired both chat and topic consumers"
    );

    let topic_count: i64 = sqlx::query_scalar("SELECT count(*) FROM topic_media")
        .fetch_one(&mut connection)
        .await?;
    assert_eq!(topic_count, 0);

    let topic_upload_id = Uuid::new_v4();
    let topic_media_id = Uuid::new_v4();
    let mut topic_binding = connection.begin().await?;
    sqlx::query(
        "INSERT INTO media_uploads \
         (id, user_id, object_key, scope, target_id, content_type, byte_size, status, \
          bound_topic_media_id, confirmed_at, consumed_at, expires_at, created_at) \
         VALUES ($1, $2, 'topics/bound', 'topic', $3, 'image/jpeg', 42, 'bound', $4, \
                 statement_timestamp(), statement_timestamp(), \
                 statement_timestamp() + interval '15 minutes', statement_timestamp())",
    )
    .bind(topic_upload_id)
    .bind(user_id)
    .bind(topic_id)
    .bind(topic_media_id)
    .execute(&mut *topic_binding)
    .await?;
    sqlx::query(
        "INSERT INTO topic_media \
         (id, topic_id, media_upload_id, type, object_key, width, height, byte_size) \
         VALUES ($1, $2, $3, 'image/jpeg', 'topics/bound', 1, 1, 42)",
    )
    .bind(topic_media_id)
    .bind(topic_id)
    .bind(topic_upload_id)
    .execute(&mut *topic_binding)
    .await?;
    topic_binding.commit().await?;

    let canonical_topic_binding: (Uuid, Uuid) = sqlx::query_as(
        "SELECT tm.media_upload_id, mu.bound_topic_media_id \
         FROM topic_media tm \
         JOIN media_uploads mu ON mu.id = tm.media_upload_id \
         WHERE tm.id = $1",
    )
    .bind(topic_media_id)
    .fetch_one(&mut connection)
    .await?;
    assert_eq!(canonical_topic_binding, (topic_upload_id, topic_media_id));

    connection.close().await?;
    database.dispose().await
}

#[tokio::test]
async fn a_failed_0006_upgrade_rolls_back_every_media_relation_and_topic_binding() -> TestResult {
    let database = TestDatabase::migrated_to(5).await?;
    let fixture_dir = env::temp_dir().join(format!(
        "jamye-server-task-8-forced-migration-{}",
        Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&fixture_dir)?;
    for migration in [
        "0001_core_reliable_messaging.sql",
        "0002_auth_sessions.sql",
        "0003_invites.sql",
        "0004_chatroom_reads.sql",
        "0005_topics.sql",
    ] {
        fs::copy(
            format!("migrations/{migration}"),
            fixture_dir.join(migration),
        )?;
    }
    let media_sql = fs::read_to_string(MEDIA_MIGRATION)?;
    fs::write(
        fixture_dir.join("0006_forced_media_failure.sql"),
        format!("{media_sql}\n\nSELECT * FROM task_8_relation_that_must_not_exist;\n"),
    )?;

    let result: TestResult = async {
        let migrator = sqlx::migrate::Migrator::new(fixture_dir.as_path()).await?;
        let mut connection = database.connection().await?;
        let migration = migrator.run_to(6, &mut connection).await;
        assert!(
            migration.is_err(),
            "forced task-8 migration unexpectedly passed"
        );
        for table in ["media_uploads", "message_media"] {
            assert!(!table_exists(&mut connection, table).await?);
        }
        assert!(!column_exists(&mut connection, "topic_media", "media_upload_id").await?);
        for constraint in ["uq_topic_media_upload", "fk_topic_media_bound_upload"] {
            assert!(!constraint_exists(&mut connection, constraint).await?);
        }
        let applied: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM _sqlx_migrations WHERE success AND version = 6",
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

async fn insert_bound_chat_attachment(
    connection: &mut sqlx::PgConnection,
    upload_id: Uuid,
    user_id: Uuid,
    chatroom_id: Uuid,
    message_id: Uuid,
    position: i32,
) -> TestResult {
    let object_key = format!("chat/{chatroom_id}/{upload_id}");
    insert_bound_chat_upload(
        connection,
        upload_id,
        user_id,
        chatroom_id,
        message_id,
        &object_key,
    )
    .await?;
    sqlx::query(
        "INSERT INTO message_media \
         (id, message_id, media_upload_id, type, object_key, byte_size, position) \
         VALUES ($1, $2, $3, 'image/jpeg', $4, 42, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(message_id)
    .bind(upload_id)
    .bind(object_key)
    .bind(position)
    .execute(connection)
    .await?;
    Ok(())
}

async fn insert_bound_chat_upload(
    connection: &mut sqlx::PgConnection,
    upload_id: Uuid,
    user_id: Uuid,
    chatroom_id: Uuid,
    message_id: Uuid,
    object_key: &str,
) -> TestResult {
    sqlx::query(
        "INSERT INTO media_uploads \
         (id, user_id, object_key, scope, target_id, content_type, byte_size, status, \
          bound_message_id, confirmed_at, consumed_at, expires_at, created_at) \
         VALUES ($1, $2, $3, 'chat', $4, 'image/jpeg', 42, 'bound', $5, \
                 statement_timestamp(), statement_timestamp(), \
                 statement_timestamp() + interval '15 minutes', statement_timestamp())",
    )
    .bind(upload_id)
    .bind(user_id)
    .bind(object_key)
    .bind(chatroom_id)
    .bind(message_id)
    .execute(connection)
    .await?;
    Ok(())
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

async fn column_exists(
    connection: &mut sqlx::PgConnection,
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
