//! Fresh disposable migration-chain coverage owned by Task-12.

use std::io;

use sqlx::{Connection, PgConnection};

use crate::{TestResult, postgres_support::TestDatabase};

#[tokio::test]
async fn fresh_disposable_database_applies_the_canonical_0001_through_0008_chain() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let mut connection = database.connection().await?;
    let result: TestResult = async {
        let applied: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM _sqlx_migrations WHERE success AND version BETWEEN 1 AND 8",
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
            8,
            "fresh disposable chain did not apply 0001 through 0008",
        )?;
        require_eq(latest, 8, "fresh disposable chain did not end at 0008")?;
        Ok(())
    }
    .await;

    connection.close().await?;
    database.dispose().await?;
    result
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
