use std::{sync::Arc, time::Duration};

use jamye_server::{
    adapters::postgres::{
        messaging::PostgresMessagingRepository, transactions::SqlxTransactionManager,
    },
    application::{
        auth::AccessIdentity,
        messaging::{MessagingService, SendMessageInput},
        topics::{TopicCreateInput, TopicsError},
    },
    ports::{topics::CreateTopicOutcome, transactions::TransactionManager},
};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    TestResult,
    postgres_support::TestDatabase,
    topic_helpers::{groups_service, harness, topology},
};

#[tokio::test]
async fn t1_is_atomic_idempotent_and_emits_distinct_bootstrap_and_announcement_events() -> TestResult
{
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let harness = harness(pool.clone());
    let key = Uuid::new_v4();

    let created = harness
        .service
        .create_topic(
            fixture.author_id,
            fixture.group_id,
            TopicCreateInput {
                idempotency_key: key,
                title: "  원자 주제  ".to_owned(),
            },
        )
        .await?;
    let topic = match created {
        CreateTopicOutcome::Created(topic) => topic,
        CreateTopicOutcome::Existing(_) => return Err("first topic create was not new".into()),
    };
    assert_eq!(topic.title, "원자 주제");
    assert_eq!(topic.author_nickname, "주제 작성자");
    assert!(!topic.unread);

    let events = sqlx::query_as::<_, (Uuid, i64, Uuid, String, Value)>(
        "SELECT id, cursor, conversation_id, event_type, payload \
         FROM conversation_events ORDER BY cursor",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].3, "topic.created");
    assert_eq!(events[0].2, topic.chatroom_id);
    assert_eq!(events[0].4["topic_id"], topic.id.to_string());
    assert_eq!(events[1].3, "message.created");
    assert_eq!(events[1].2, fixture.main_chatroom_id);
    assert_ne!(events[0].0, events[1].0);
    assert!(events[0].1 < events[1].1);

    let outbox = sqlx::query_as::<_, (Uuid, String, Uuid, Uuid)>(
        "SELECT id, event_type, aggregate_id, conversation_event_id \
         FROM outbox_events ORDER BY created_at, id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(outbox.len(), 2);
    assert_ne!(outbox[0].0, outbox[1].0);
    assert!(
        outbox
            .iter()
            .any(|row| row.1 == "topic.created" && row.2 == topic.chatroom_id)
    );
    assert!(
        outbox
            .iter()
            .any(|row| row.1 == "message.created" && row.2 == fixture.main_chatroom_id)
    );
    assert_ne!(outbox[0].3, outbox[1].3);

    let marker: (Uuid, i64) = sqlx::query_as(
        "SELECT chatroom_id, last_read_cursor FROM chatroom_reads WHERE user_id = $1",
    )
    .bind(fixture.author_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(marker, (topic.chatroom_id, events[0].1));
    let announcement: (Uuid, String, String) =
        sqlx::query_as("SELECT sender_id, type, body FROM messages WHERE chatroom_id = $1")
            .bind(fixture.main_chatroom_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(announcement.0, fixture.author_id);
    assert_eq!(announcement.1, "user");
    assert_eq!(
        announcement.2,
        format!(
            "새로운 주제를 올렸어요: [원자 주제](/groups/{}/topics/{}/chat)",
            fixture.group_id, topic.id
        )
    );

    let retried = harness
        .service
        .create_topic(
            fixture.author_id,
            fixture.group_id,
            TopicCreateInput {
                idempotency_key: key,
                title: "원자 주제".to_owned(),
            },
        )
        .await?;
    match retried {
        CreateTopicOutcome::Existing(existing) => assert_eq!(existing.id, topic.id),
        CreateTopicOutcome::Created(_) => return Err("same request created a duplicate".into()),
    }
    assert_eq!(counts(&pool).await?, (1, 1, 2, 2, 1, 1));

    assert_eq!(
        harness
            .service
            .create_topic(
                fixture.author_id,
                fixture.group_id,
                TopicCreateInput {
                    idempotency_key: key,
                    title: "다른 제목".to_owned(),
                },
            )
            .await,
        Err(TopicsError::IdempotencyConflict)
    );
    assert_eq!(counts(&pool).await?, (1, 1, 2, 2, 1, 1));

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn a_late_t1_failure_rolls_back_every_core_row() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let harness = harness(pool.clone());
    sqlx::query(
        "CREATE FUNCTION task7_reject_announcement_outbox() RETURNS trigger \
         LANGUAGE plpgsql AS $$ BEGIN \
             IF NEW.event_type = 'message.created' THEN \
                 RAISE EXCEPTION 'task-7 forced late failure'; \
             END IF; \
             RETURN NEW; \
         END $$",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TRIGGER task7_fail_announcement_outbox \
         BEFORE INSERT ON outbox_events FOR EACH ROW \
         EXECUTE FUNCTION task7_reject_announcement_outbox()",
    )
    .execute(&pool)
    .await?;

    assert_eq!(
        harness
            .service
            .create_topic(
                fixture.author_id,
                fixture.group_id,
                TopicCreateInput {
                    idempotency_key: Uuid::new_v4(),
                    title: "롤백 주제".to_owned(),
                },
            )
            .await,
        Err(TopicsError::DatabaseUnavailable)
    );
    assert_eq!(counts(&pool).await?, (0, 0, 0, 0, 0, 0));

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn t1_and_group_soft_delete_are_serialized_in_both_commit_orders() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let topics = harness(pool.clone());
    let groups = groups_service(pool.clone())?;
    let transactions = SqlxTransactionManager::new(pool.clone());

    let topic_first = topology(&pool).await?;
    let topic_first_group_id = topic_first.group_id;
    let topic_first_owner_id = topic_first.owner_id;
    let topic_first_author_id = topic_first.author_id;
    let mut topic_transaction = transactions.begin().await?;
    let created = topics
        .service
        .create_topic_in_transaction(
            topic_transaction.as_mut(),
            topic_first_author_id,
            topic_first_group_id,
            TopicCreateInput {
                idempotency_key: Uuid::new_v4(),
                title: "생성 우선".to_owned(),
            },
        )
        .await?
        .into_topic();
    let delete_service = groups.clone();
    let mut delete_after_topic = Box::pin(async move {
        delete_service
            .delete_group(topic_first_owner_id, topic_first_group_id)
            .await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut delete_after_topic)
            .await
            .is_err(),
        "group delete bypassed the uncommitted T1 group lock"
    );
    transactions.commit(topic_transaction).await?;
    assert_eq!(delete_after_topic.await, Ok(()));
    assert_eq!(
        group_counts(&pool, topic_first_group_id).await?,
        (1, 1, 2, 2, 1, 1)
    );
    assert_eq!(
        topics
            .service
            .get_topic(topic_first_author_id, topic_first_group_id, created.id)
            .await,
        Err(TopicsError::GroupNotFound)
    );

    let delete_first = topology(&pool).await?;
    let delete_first_group_id = delete_first.group_id;
    let delete_first_owner_id = delete_first.owner_id;
    let delete_first_author_id = delete_first.author_id;
    let mut delete_transaction = transactions.begin().await?;
    groups
        .delete_group_in_transaction(
            delete_transaction.as_mut(),
            delete_first_owner_id,
            delete_first_group_id,
        )
        .await?;
    let create_service = topics.service.clone();
    let mut create_after_delete = Box::pin(async move {
        create_service
            .create_topic(
                delete_first_author_id,
                delete_first_group_id,
                TopicCreateInput {
                    idempotency_key: Uuid::new_v4(),
                    title: "삭제 우선".to_owned(),
                },
            )
            .await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut create_after_delete)
            .await
            .is_err(),
        "T1 bypassed the uncommitted group-delete lock"
    );
    transactions.commit(delete_transaction).await?;
    assert_eq!(create_after_delete.await, Err(TopicsError::GroupNotFound));
    assert_eq!(
        group_counts(&pool, delete_first_group_id).await?,
        (0, 0, 0, 0, 0, 0)
    );

    let source = std::fs::read_to_string("src/adapters/postgres/topics/mutation.rs")?;
    let group_lock = source
        .find("SELECT id FROM groups")
        .ok_or_else(|| std::io::Error::other("topic create omitted the live-group lock"))?;
    let membership = source
        .find("SELECT membership.role")
        .ok_or_else(|| std::io::Error::other("topic create omitted membership validation"))?;
    assert!(group_lock < membership);
    assert!(source[group_lock..membership].contains("FOR UPDATE"));

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn first_topic_chat_message_adds_only_the_original_user_message() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let topics = harness(pool.clone());
    let topic = crate::topic_helpers::create_topic(
        &topics,
        fixture.author_id,
        fixture.group_id,
        Uuid::new_v4(),
        "첫 메시지 주제",
    )
    .await?;
    let transactions = Arc::new(SqlxTransactionManager::new(pool.clone()));
    let messages = Arc::new(PostgresMessagingRepository::new(pool.clone()));
    let messaging = MessagingService::new(transactions, messages);
    messaging
        .send_message(
            &AccessIdentity::new(fixture.member_id, Uuid::nil(), "task-7-test"),
            SendMessageInput {
                chatroom_id: topic.chatroom_id,
                client_msg_id: Uuid::new_v4(),
                body: Some("첫 대화".to_owned()),
                media_upload_ids: Vec::new(),
                idempotency_key: None,
            },
        )
        .await?;
    let rows = sqlx::query_as::<_, (String, Option<Uuid>, Option<String>)>(
        "SELECT type, sender_id, body FROM messages WHERE chatroom_id = $1 ORDER BY created_at, id",
    )
    .bind(topic.chatroom_id)
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        rows,
        vec![(
            "user".to_owned(),
            Some(fixture.member_id),
            Some("첫 대화".to_owned())
        )]
    );

    pool.close().await;
    database.dispose().await
}

async fn counts(pool: &sqlx::PgPool) -> TestResult<(i64, i64, i64, i64, i64, i64)> {
    let topics = sqlx::query_scalar("SELECT count(*) FROM topics")
        .fetch_one(pool)
        .await?;
    let topic_chatrooms = sqlx::query_scalar("SELECT count(*) FROM chatrooms WHERE type = 'topic'")
        .fetch_one(pool)
        .await?;
    let events = sqlx::query_scalar("SELECT count(*) FROM conversation_events")
        .fetch_one(pool)
        .await?;
    let outbox = sqlx::query_scalar("SELECT count(*) FROM outbox_events")
        .fetch_one(pool)
        .await?;
    let reads = sqlx::query_scalar("SELECT count(*) FROM chatroom_reads")
        .fetch_one(pool)
        .await?;
    let messages = sqlx::query_scalar("SELECT count(*) FROM messages")
        .fetch_one(pool)
        .await?;
    Ok((topics, topic_chatrooms, events, outbox, reads, messages))
}

async fn group_counts(
    pool: &sqlx::PgPool,
    group_id: Uuid,
) -> TestResult<(i64, i64, i64, i64, i64, i64)> {
    let topics = sqlx::query_scalar("SELECT count(*) FROM topics WHERE group_id = $1")
        .bind(group_id)
        .fetch_one(pool)
        .await?;
    let topic_chatrooms =
        sqlx::query_scalar("SELECT count(*) FROM chatrooms WHERE group_id = $1 AND type = 'topic'")
            .bind(group_id)
            .fetch_one(pool)
            .await?;
    let events = sqlx::query_scalar(
        "SELECT count(*) FROM conversation_events event \
         JOIN chatrooms chatroom ON chatroom.id = event.conversation_id \
         WHERE chatroom.group_id = $1",
    )
    .bind(group_id)
    .fetch_one(pool)
    .await?;
    let outbox = sqlx::query_scalar(
        "SELECT count(*) FROM outbox_events event \
         JOIN chatrooms chatroom ON chatroom.id = event.aggregate_id \
         WHERE chatroom.group_id = $1",
    )
    .bind(group_id)
    .fetch_one(pool)
    .await?;
    let reads = sqlx::query_scalar(
        "SELECT count(*) FROM chatroom_reads marker \
         JOIN chatrooms chatroom ON chatroom.id = marker.chatroom_id \
         WHERE chatroom.group_id = $1",
    )
    .bind(group_id)
    .fetch_one(pool)
    .await?;
    let messages = sqlx::query_scalar(
        "SELECT count(*) FROM messages message \
         JOIN chatrooms chatroom ON chatroom.id = message.chatroom_id \
         WHERE chatroom.group_id = $1",
    )
    .bind(group_id)
    .fetch_one(pool)
    .await?;
    Ok((topics, topic_chatrooms, events, outbox, reads, messages))
}
