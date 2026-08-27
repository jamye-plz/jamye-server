use std::collections::BTreeMap;

use jamye_server::{
    adapters::postgres::{
        notifications::PostgresNotificationsRepository, transactions::SqlxTransactionManager,
    },
    ports::{
        push::{
            ListNotificationsQuery, MarkNotificationReadCommand, NotificationType,
            NotificationsRepository, NotificationsRepositoryError,
        },
        transactions::TransactionManager,
    },
};
use serde_json::{Value, json};
use sqlx::PgPool;
use time::{Duration as TimeDuration, OffsetDateTime};
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

#[tokio::test]
async fn postgres_n1_pages_newest_first_without_gaps_and_counts_all_unread_rows() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let actor_id = insert_user(&pool, "알림 사용자").await?;
    let other_id = insert_user(&pool, "다른 사용자").await?;
    let mut notifications = Vec::new();
    for sequence in 1..=5 {
        notifications.push(insert_notification(&pool, actor_id, sequence, sequence == 2).await?);
    }
    let foreign_cursor = insert_notification(&pool, other_id, 99, false).await?;
    let repository = PostgresNotificationsRepository::new(pool.clone());

    let first = repository
        .list_notifications(ListNotificationsQuery {
            user_id: actor_id,
            after: None,
            limit: 2,
        })
        .await?;
    assert_eq!(ids(&first.items), vec![notifications[4], notifications[3]]);
    assert_eq!(first.next_cursor, Some(notifications[3].to_string()));
    assert_eq!(first.unread_count, 4);
    assert_eq!(first.items[0].notification_type, NotificationType::Other);
    assert_eq!(
        first.items[0].args,
        BTreeMap::from([("sequence".to_owned(), Value::from(5))])
    );

    let second = repository
        .list_notifications(ListNotificationsQuery {
            user_id: actor_id,
            after: Some(notifications[3]),
            limit: 2,
        })
        .await?;
    assert_eq!(ids(&second.items), vec![notifications[2], notifications[1]]);
    assert_eq!(second.next_cursor, Some(notifications[1].to_string()));
    assert_eq!(second.unread_count, 4);

    let third = repository
        .list_notifications(ListNotificationsQuery {
            user_id: actor_id,
            after: Some(notifications[1]),
            limit: 2,
        })
        .await?;
    assert_eq!(ids(&third.items), vec![notifications[0]]);
    assert_eq!(third.next_cursor, None);
    assert_eq!(third.unread_count, 4);

    assert_eq!(
        repository
            .list_notifications(ListNotificationsQuery {
                user_id: actor_id,
                after: Some(foreign_cursor),
                limit: 2,
            })
            .await,
        Err(NotificationsRepositoryError::CursorInvalid)
    );

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn postgres_n2_is_owner_scoped_idempotent_and_mutates_no_other_read_projection() -> TestResult
{
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let actor_id = insert_user(&pool, "읽음 사용자").await?;
    let other_id = insert_user(&pool, "외부 사용자").await?;
    let target_id = insert_notification(&pool, actor_id, 1, false).await?;
    let sibling_id = insert_notification(&pool, actor_id, 2, false).await?;
    let foreign_id = insert_notification(&pool, other_id, 3, false).await?;
    let repository = PostgresNotificationsRepository::new(pool.clone());
    let transactions = SqlxTransactionManager::new(pool.clone());

    let mut first_transaction = transactions.begin().await?;
    let first = repository
        .mark_notification_read(
            first_transaction.as_mut(),
            &MarkNotificationReadCommand {
                user_id: actor_id,
                notification_id: target_id,
            },
        )
        .await?;
    transactions.commit(first_transaction).await?;

    let mut retry_transaction = transactions.begin().await?;
    let retry = repository
        .mark_notification_read(
            retry_transaction.as_mut(),
            &MarkNotificationReadCommand {
                user_id: actor_id,
                notification_id: target_id,
            },
        )
        .await?;
    transactions.commit(retry_transaction).await?;
    assert_eq!(retry, first);

    for inaccessible_id in [Uuid::new_v4(), foreign_id] {
        let mut transaction = transactions.begin().await?;
        let result = repository
            .mark_notification_read(
                transaction.as_mut(),
                &MarkNotificationReadCommand {
                    user_id: actor_id,
                    notification_id: inaccessible_id,
                },
            )
            .await;
        transactions.rollback(transaction).await?;
        assert_eq!(
            result,
            Err(NotificationsRepositoryError::NotificationNotFound)
        );
    }

    let stored_read_at = sqlx::query_scalar::<_, Option<OffsetDateTime>>(
        "SELECT read_at FROM notifications WHERE id = $1",
    )
    .bind(target_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(stored_read_at, Some(first.read_at));
    let sibling_read_at = sqlx::query_scalar::<_, Option<OffsetDateTime>>(
        "SELECT read_at FROM notifications WHERE id = $1",
    )
    .bind(sibling_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(sibling_read_at, None);
    let chatroom_read_count: i64 = sqlx::query_scalar("SELECT count(*) FROM chatroom_reads")
        .fetch_one(&pool)
        .await?;
    assert_eq!(chatroom_read_count, 0);

    pool.close().await;
    database.dispose().await
}

fn ids(items: &[jamye_server::ports::push::NotificationRecord]) -> Vec<Uuid> {
    items.iter().map(|item| item.id).collect()
}

async fn insert_user(pool: &PgPool, nickname: &str) -> TestResult<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, $2)")
        .bind(id)
        .bind(nickname)
        .execute(pool)
        .await?;
    Ok(id)
}

async fn insert_notification(
    pool: &PgPool,
    user_id: Uuid,
    sequence: i64,
    read: bool,
) -> TestResult<Uuid> {
    let id = Uuid::new_v4();
    let created_at = OffsetDateTime::UNIX_EPOCH + TimeDuration::seconds(sequence * 10);
    let read_at = read.then_some(created_at + TimeDuration::seconds(1));
    sqlx::query(
        "INSERT INTO notifications (id, user_id, type, payload, read_at, created_at) \
         VALUES ($1, $2, 'other', $3, $4, $5)",
    )
    .bind(id)
    .bind(user_id)
    .bind(json!({"sequence": sequence}))
    .bind(read_at)
    .bind(created_at)
    .execute(pool)
    .await?;
    Ok(id)
}
