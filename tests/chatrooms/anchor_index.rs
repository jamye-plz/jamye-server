use std::path::Path;

use serde_json::Value;
use sqlx::{AssertSqlSafe, Connection, PgPool, SqlSafeStr};
use uuid::Uuid;

use jamye_server::application::chatrooms::{ChatroomsError, ReadAnchorInput};

use crate::{
    TestResult,
    chatroom_helpers::{harness, insert_message_created_event, insert_user_message, topology},
    postgres_support::TestDatabase,
};

const INDEX: &str = "ix_conversation_events_message_anchor";

#[tokio::test]
async fn failed_anchor_index_migration_rolls_back_and_can_retry_from_v8() -> TestResult {
    let database = TestDatabase::migrated_to(8).await?;
    let mut connection = database.connection().await?;
    let migrator = sqlx::migrate::Migrator::new(Path::new("migrations")).await?;
    let migrations = migrator
        .iter()
        .map(|migration| {
            if migration.version != 9 {
                return migration.clone();
            }
            assert!(!migration.no_tx);
            sqlx::migrate::Migration::new(
                migration.version,
                migration.description.clone(),
                migration.migration_type,
                AssertSqlSafe(format!("{}\nSELECT 1 / 0;", migration.sql.as_str())).into_sql_str(),
                false,
            )
        })
        .collect();
    let failed = sqlx::migrate::Migrator::with_migrations(migrations)
        .run(&mut connection)
        .await;
    let absent: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NULL")
        .bind(INDEX)
        .fetch_one(&mut connection)
        .await?;
    let recorded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version = 9")
            .fetch_one(&mut connection)
            .await?;
    migrator.run(&mut connection).await?;
    let retried: bool =
        sqlx::query_scalar("SELECT indisvalid FROM pg_index WHERE indexrelid = to_regclass($1)")
            .bind(INDEX)
            .fetch_one(&mut connection)
            .await?;
    connection.close().await?;
    database.dispose().await?;
    assert!(failed.is_err());
    assert!(absent);
    assert_eq!(recorded, 0);
    assert!(retried);
    Ok(())
}

async fn anchor_plan(pool: &PgPool, room: Uuid, message: Uuid) -> TestResult<Value> {
    Ok(sqlx::query_scalar(
        "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)
         SELECT event.cursor FROM messages AS message
         JOIN conversation_events AS event ON event.conversation_id = message.chatroom_id
           AND event.event_type = 'message.created' AND event.event_version = 1
           AND event.payload ->> 'id' = message.id::text
         WHERE message.id = $1 AND message.chatroom_id = $2
         ORDER BY event.cursor FOR SHARE OF message, event",
    )
    .bind(message)
    .bind(room)
    .fetch_one(pool)
    .await?)
}

#[tokio::test]
async fn anchor_index_upgrades_v8_and_indexes_the_real_lookup_without_rejecting_ambiguity()
-> TestResult {
    let database = TestDatabase::migrated_to(8).await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    sqlx::query(
        "INSERT INTO conversation_events (id, conversation_id, event_type, event_version, payload)
         SELECT gen_random_uuid(), $1, 'message.created', 1,
                jsonb_build_object('id', gen_random_uuid()::text, 'body', repeat('x', 128))
         FROM generate_series(1, 6000)",
    )
    .bind(fixture.chatroom_id)
    .execute(&pool)
    .await?;
    let message = insert_user_message(
        &pool,
        fixture.chatroom_id,
        fixture.owner_id,
        "indexed anchor",
        time::OffsetDateTime::now_utc(),
    )
    .await?;
    let cursor = insert_message_created_event(&pool, fixture.chatroom_id, message).await?;
    sqlx::query("ANALYZE conversation_events")
        .execute(&pool)
        .await?;
    let before = anchor_plan(&pool, fixture.chatroom_id, message).await?;
    let mut connection = database.connection().await?;
    let migrator = sqlx::migrate::Migrator::new(Path::new("migrations")).await?;
    migrator.run(&mut connection).await?;
    migrator.run(&mut connection).await?;
    connection.close().await?;
    sqlx::query("ANALYZE conversation_events")
        .execute(&pool)
        .await?;
    let after = anchor_plan(&pool, fixture.chatroom_id, message).await?;
    let index_valid: Option<bool> = sqlx::query_scalar(
        "SELECT indisvalid AND indisready FROM pg_index WHERE indexrelid = to_regclass($1)",
    )
    .bind(INDEX)
    .fetch_optional(&pool)
    .await?;
    let service = harness(pool.clone()).service;
    let marker = service
        .mark_read_anchor(
            fixture.owner_id,
            fixture.chatroom_id,
            ReadAnchorInput::MessageId(message),
        )
        .await?;
    // Keep the existing C3 ambiguity rejection: this is a non-unique accelerator,
    // not a change to event ingestion or an implicit historical-data rewrite.
    sqlx::query(
        "INSERT INTO conversation_events (id, conversation_id, event_type, event_version, payload)
         VALUES (gen_random_uuid(), $1, 'message.created', 1, jsonb_build_object('id', $2::text))",
    )
    .bind(fixture.chatroom_id)
    .bind(message.to_string())
    .execute(&pool)
    .await?;
    let duplicates: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM conversation_events WHERE conversation_id = $1
         AND event_type = 'message.created' AND event_version = 1 AND payload ->> 'id' = $2",
    )
    .bind(fixture.chatroom_id)
    .bind(message.to_string())
    .fetch_one(&pool)
    .await?;
    let ambiguous = service
        .mark_read_anchor(
            fixture.owner_id,
            fixture.chatroom_id,
            ReadAnchorInput::MessageId(message),
        )
        .await;
    let index_bytes: Option<i64> = sqlx::query_scalar("SELECT pg_relation_size(to_regclass($1))")
        .bind(INDEX)
        .fetch_one(&pool)
        .await?;
    println!("ANCHOR_PLAN_BEFORE: {before}");
    println!("ANCHOR_PLAN_AFTER: {after}");
    println!("ANCHOR_INDEX_BYTES: {index_bytes:?}");
    pool.close().await;
    database.dispose().await?;
    assert_eq!(
        index_valid,
        Some(true),
        "message anchor index must be installed and valid"
    );
    assert!(
        after.to_string().contains(INDEX),
        "default PostgreSQL planner must select the anchor index"
    );
    assert!(!before.to_string().contains(INDEX));
    assert_eq!(duplicates, 2);
    assert_eq!(marker.last_read_cursor, cursor);
    assert_eq!(ambiguous, Err(ChatroomsError::RequestValidation));
    Ok(())
}
