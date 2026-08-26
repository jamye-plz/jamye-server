use std::{io, sync::Arc};

use jamye_server::{
    adapters::postgres::transactions::SqlxTransactionManager,
    application::chatrooms::{ChatroomsError, ReadCursorInput},
    ports::transactions::TransactionManager,
};
use tokio::sync::Barrier;
use uuid::Uuid;

use crate::{
    TestResult,
    chatroom_helpers::{harness, insert_event, topology},
    postgres_support::TestDatabase,
};

#[tokio::test]
async fn unknown_cross_conversation_and_nonmember_reads_mutate_nothing() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let service = harness(pool.clone()).service;
    let valid_cursor = insert_event(&pool, fixture.chatroom_id).await?;
    let foreign_cursor = insert_event(&pool, fixture.other_chatroom_id).await?;

    let unknown = service
        .mark_read(
            fixture.owner_id,
            fixture.chatroom_id,
            ReadCursorInput {
                cursor: (foreign_cursor + 10_000).to_string(),
            },
        )
        .await;
    assert_eq!(unknown, Err(ChatroomsError::RequestValidation));

    let foreign = service
        .mark_read(
            fixture.owner_id,
            fixture.chatroom_id,
            ReadCursorInput {
                cursor: foreign_cursor.to_string(),
            },
        )
        .await;
    assert_eq!(foreign, Err(ChatroomsError::RequestValidation));

    let outsider = service
        .mark_read(
            fixture.outsider_id,
            fixture.chatroom_id,
            ReadCursorInput {
                cursor: valid_cursor.to_string(),
            },
        )
        .await;
    assert_eq!(outsider, Err(ChatroomsError::MembershipRequired));
    let marker_count: i64 = sqlx::query_scalar("SELECT count(*) FROM chatroom_reads")
        .fetch_one(&pool)
        .await?;
    assert_eq!(marker_count, 0);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn stale_duplicate_and_newer_reads_return_one_monotonic_canonical_marker() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let service = harness(pool.clone()).service;
    let first_cursor = insert_event(&pool, fixture.chatroom_id).await?;
    let second_cursor = insert_event(&pool, fixture.chatroom_id).await?;

    let first = service
        .mark_read(
            fixture.owner_id,
            fixture.chatroom_id,
            ReadCursorInput {
                cursor: first_cursor.to_string(),
            },
        )
        .await?;
    let duplicate = service
        .mark_read(
            fixture.owner_id,
            fixture.chatroom_id,
            ReadCursorInput {
                cursor: first_cursor.to_string(),
            },
        )
        .await?;
    assert_eq!(duplicate.id, first.id);
    assert_eq!(duplicate.last_read_cursor, first_cursor);
    assert_eq!(duplicate.updated_at, first.updated_at);

    let advanced = service
        .mark_read(
            fixture.owner_id,
            fixture.chatroom_id,
            ReadCursorInput {
                cursor: second_cursor.to_string(),
            },
        )
        .await?;
    assert_eq!(advanced.id, first.id);
    assert_eq!(advanced.last_read_cursor, second_cursor);
    assert!(advanced.updated_at >= first.updated_at);

    let stale = service
        .mark_read(
            fixture.owner_id,
            fixture.chatroom_id,
            ReadCursorInput {
                cursor: first_cursor.to_string(),
            },
        )
        .await?;
    assert_eq!(stale, advanced);
    assert_eq!(
        service
            .read_marker(fixture.owner_id, fixture.chatroom_id)
            .await?,
        Some(advanced.clone())
    );
    assert_eq!(
        service
            .read_marker(fixture.outsider_id, fixture.chatroom_id)
            .await,
        Err(ChatroomsError::MembershipRequired)
    );
    let marker_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chatroom_reads WHERE user_id = $1 AND chatroom_id = $2",
    )
    .bind(fixture.owner_id)
    .bind(fixture.chatroom_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(marker_count, 1);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn concurrent_reads_converge_on_the_highest_cursor_and_one_row() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let service = harness(pool.clone()).service;
    let mut cursors = Vec::new();
    for _ in 0..8 {
        cursors.push(insert_event(&pool, fixture.chatroom_id).await?);
    }
    let highest = cursors
        .iter()
        .copied()
        .max()
        .ok_or_else(|| io::Error::other("missing cursor"))?;
    let barrier = Arc::new(Barrier::new(cursors.len()));
    let mut tasks = Vec::new();
    for cursor in cursors {
        let service = service.clone();
        let barrier = barrier.clone();
        let user_id = fixture.owner_id;
        let chatroom_id = fixture.chatroom_id;
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            service
                .mark_read(
                    user_id,
                    chatroom_id,
                    ReadCursorInput {
                        cursor: cursor.to_string(),
                    },
                )
                .await
        }));
    }
    for task in tasks {
        task.await??;
    }

    let canonical = service
        .read_marker(fixture.owner_id, fixture.chatroom_id)
        .await?
        .ok_or_else(|| io::Error::other("missing canonical read marker"))?;
    assert_eq!(canonical.last_read_cursor, highest);
    let rows: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT id, last_read_cursor FROM chatroom_reads \
         WHERE user_id = $1 AND chatroom_id = $2",
    )
    .bind(fixture.owner_id)
    .bind(fixture.chatroom_id)
    .fetch_all(&pool)
    .await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, highest);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn base_read_uses_the_caller_transaction_and_standalone_wrapper_commits() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let service = harness(pool.clone()).service;
    let cursor = insert_event(&pool, fixture.chatroom_id).await?;
    let manager = SqlxTransactionManager::new(pool.clone());
    let mut transaction = manager.begin().await?;
    let marker = service
        .mark_read_in_transaction(
            transaction.as_mut(),
            fixture.owner_id,
            fixture.chatroom_id,
            cursor,
        )
        .await?;
    assert_eq!(marker.last_read_cursor, cursor);
    manager.rollback(transaction).await?;
    let rolled_back: i64 = sqlx::query_scalar("SELECT count(*) FROM chatroom_reads")
        .fetch_one(&pool)
        .await?;
    assert_eq!(rolled_back, 0);

    let committed = service
        .mark_read(
            fixture.owner_id,
            fixture.chatroom_id,
            ReadCursorInput {
                cursor: cursor.to_string(),
            },
        )
        .await?;
    assert_eq!(committed.last_read_cursor, cursor);
    let committed_count: i64 = sqlx::query_scalar("SELECT count(*) FROM chatroom_reads")
        .fetch_one(&pool)
        .await?;
    assert_eq!(committed_count, 1);

    pool.close().await;
    database.dispose().await
}
