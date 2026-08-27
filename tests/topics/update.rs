use std::sync::Arc;

use jamye_server::{
    adapters::postgres::{
        chatrooms::PostgresChatroomsRepository, messaging::PostgresMessagingRepository,
        transactions::SqlxTransactionManager,
    },
    application::{
        auth::AccessIdentity,
        chatrooms::{ChatroomsService, ReadCursorInput},
        messaging::{MessagingService, SendMessageInput},
        topics::{
            TopicPageInput, TopicPatchInput, TopicTagInput, TopicTagPageInput, TopicTagsInput,
            TopicsError,
        },
    },
    ports::{topics::TopicStatus, transactions::TransactionManager},
};
use uuid::Uuid;

use crate::{
    TestResult,
    postgres_support::TestDatabase,
    topic_helpers::{create_topic, harness, topology},
};

#[tokio::test]
async fn unread_tracks_monotonic_server_cursors_without_topic_count_queries() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let topics = harness(pool.clone());
    let topic = create_topic(
        &topics,
        fixture.author_id,
        fixture.group_id,
        Uuid::new_v4(),
        "읽음 주제",
    )
    .await?;
    let bootstrap: i64 = sqlx::query_scalar(
        "SELECT cursor FROM conversation_events \
         WHERE conversation_id = $1 AND event_type = 'topic.created'",
    )
    .bind(topic.chatroom_id)
    .fetch_one(&pool)
    .await?;

    let author_page = topics
        .service
        .list_topics(
            fixture.author_id,
            fixture.group_id,
            TopicPageInput {
                after: None,
                limit: None,
                date: None,
            },
        )
        .await?;
    assert!(!author_page.items[0].unread);
    let member_page = topics
        .service
        .list_topics(
            fixture.member_id,
            fixture.group_id,
            TopicPageInput {
                after: None,
                limit: None,
                date: None,
            },
        )
        .await?;
    assert!(member_page.items[0].unread);

    let reads = ChatroomsService::new(
        Arc::new(SqlxTransactionManager::new(pool.clone())),
        Arc::new(PostgresChatroomsRepository::new(pool.clone())),
    );
    reads
        .mark_read(
            fixture.member_id,
            topic.chatroom_id,
            ReadCursorInput {
                cursor: bootstrap.to_string(),
            },
        )
        .await?;
    assert!(
        !topics
            .service
            .list_topics(
                fixture.member_id,
                fixture.group_id,
                TopicPageInput {
                    after: None,
                    limit: None,
                    date: None,
                },
            )
            .await?
            .items[0]
            .unread
    );

    let messaging = MessagingService::new(
        Arc::new(SqlxTransactionManager::new(pool.clone())),
        Arc::new(PostgresMessagingRepository::new(pool.clone())),
    );
    messaging
        .send_message(
            &AccessIdentity::new(fixture.author_id, Uuid::nil(), "task-7-test"),
            SendMessageInput {
                chatroom_id: topic.chatroom_id,
                client_msg_id: Uuid::new_v4(),
                body: Some("새 메시지".to_owned()),
                media_upload_ids: Vec::new(),
                idempotency_key: None,
            },
        )
        .await?;
    reads
        .mark_read(
            fixture.member_id,
            topic.chatroom_id,
            ReadCursorInput {
                cursor: bootstrap.to_string(),
            },
        )
        .await?;
    assert!(
        topics
            .service
            .list_topics(
                fixture.member_id,
                fixture.group_id,
                TopicPageInput {
                    after: None,
                    limit: None,
                    date: None,
                },
            )
            .await?
            .items[0]
            .unread
    );

    let source = std::fs::read_to_string("src/adapters/postgres/topics/query.rs")?;
    assert!(source.contains("event.cursor > COALESCE"));
    assert!(source.contains("topic_id = ANY($1)"));
    assert!(!source.contains("for topic in topics"));

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn t5_is_author_only_validates_trimmed_title_and_promotes_body() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let topics = harness(pool.clone());
    let topic = create_topic(
        &topics,
        fixture.author_id,
        fixture.group_id,
        Uuid::new_v4(),
        "원래 제목",
    )
    .await?;

    for actor_id in [fixture.owner_id, fixture.member_id] {
        assert_eq!(
            topics
                .service
                .patch_topic(
                    actor_id,
                    fixture.group_id,
                    topic.id,
                    TopicPatchInput {
                        title: Some("권한 없는 수정".to_owned()),
                        body: None,
                    },
                )
                .await,
            Err(TopicsError::AuthorRequired)
        );
        assert_eq!(
            topics
                .service
                .patch_topic(
                    actor_id,
                    fixture.group_id,
                    Uuid::new_v4(),
                    TopicPatchInput {
                        title: Some("존재 여부 비공개".to_owned()),
                        body: None,
                    },
                )
                .await,
            Err(TopicsError::AuthorRequired)
        );
    }
    for invalid in ["".to_owned(), "   ".to_owned(), "가".repeat(257)] {
        assert_eq!(
            topics
                .service
                .patch_topic(
                    fixture.author_id,
                    fixture.group_id,
                    topic.id,
                    TopicPatchInput {
                        title: Some(invalid),
                        body: None,
                    },
                )
                .await,
            Err(TopicsError::RequestValidation)
        );
    }
    let unchanged = topics
        .service
        .patch_topic(
            fixture.author_id,
            fixture.group_id,
            topic.id,
            TopicPatchInput {
                title: None,
                body: None,
            },
        )
        .await?;
    assert_eq!(unchanged.title, "원래 제목");
    assert_eq!(unchanged.status, TopicStatus::Seed);

    let enriched = topics
        .service
        .patch_topic(
            fixture.author_id,
            fixture.group_id,
            topic.id,
            TopicPatchInput {
                title: Some("  다듬은 제목  ".to_owned()),
                body: Some("본문".to_owned()),
            },
        )
        .await?;
    assert_eq!(enriched.title, "다듬은 제목");
    assert_eq!(enriched.body.as_deref(), Some("본문"));
    assert_eq!(enriched.status, TopicStatus::Enriched);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn t6_replaces_tags_for_author_or_owner_and_t7_paginates_for_members() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let topics = harness(pool.clone());
    let topic = create_topic(
        &topics,
        fixture.author_id,
        fixture.group_id,
        Uuid::new_v4(),
        "태그 주제",
    )
    .await?;

    assert_eq!(
        topics
            .service
            .replace_tags(
                fixture.member_id,
                fixture.group_id,
                topic.id,
                TopicTagsInput { tags: Vec::new() },
            )
            .await,
        Err(TopicsError::TopicManageRequired)
    );
    let replaced = topics
        .service
        .replace_tags(
            fixture.owner_id,
            fixture.group_id,
            topic.id,
            TopicTagsInput {
                tags: vec![
                    TopicTagInput {
                        tag: "  사용자  ".to_owned(),
                        source: "user".to_owned(),
                        confidence: None,
                    },
                    TopicTagInput {
                        tag: "AI".to_owned(),
                        source: "ai".to_owned(),
                        confidence: Some(0.9),
                    },
                ],
            },
        )
        .await?;
    assert_eq!(replaced.items.len(), 2);
    assert_eq!(replaced.items[0].tag, "AI");
    assert_eq!(replaced.items[1].tag, "사용자");

    let first = topics
        .service
        .list_tags(
            fixture.member_id,
            fixture.group_id,
            topic.id,
            TopicTagPageInput {
                after: None,
                limit: Some(1),
            },
        )
        .await?;
    assert_eq!(first.items.len(), 1);
    let cursor = first
        .next_cursor
        .ok_or_else(|| std::io::Error::other("first tag page omitted cursor"))?;
    let second = topics
        .service
        .list_tags(
            fixture.member_id,
            fixture.group_id,
            topic.id,
            TopicTagPageInput {
                after: Some(cursor),
                limit: Some(1),
            },
        )
        .await?;
    assert_eq!(second.items.len(), 1);
    assert_ne!(first.items[0].id, second.items[0].id);

    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM topic_tags WHERE topic_id = $1")
        .bind(topic.id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        topics
            .service
            .replace_tags(
                fixture.author_id,
                fixture.group_id,
                topic.id,
                TopicTagsInput {
                    tags: vec![
                        TopicTagInput {
                            tag: "중복".to_owned(),
                            source: "user".to_owned(),
                            confidence: None,
                        },
                        TopicTagInput {
                            tag: " 중복 ".to_owned(),
                            source: "ai".to_owned(),
                            confidence: Some(0.5),
                        },
                    ],
                },
            )
            .await,
        Err(TopicsError::RequestValidation)
    );
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM topic_tags WHERE topic_id = $1")
        .bind(topic.id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(before, after);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn promote_enriched_joins_the_caller_transaction_and_is_idempotent() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let topics = harness(pool.clone());
    let topic = create_topic(
        &topics,
        fixture.author_id,
        fixture.group_id,
        Uuid::new_v4(),
        "사진 예정 주제",
    )
    .await?;
    let manager = SqlxTransactionManager::new(pool.clone());

    let mut rolled_back = manager.begin().await?;
    assert_eq!(
        topics
            .service
            .promote_enriched(rolled_back.as_mut(), topic.id)
            .await?,
        TopicStatus::Enriched
    );
    manager.rollback(rolled_back).await?;
    let status: String = sqlx::query_scalar("SELECT status FROM topics WHERE id = $1")
        .bind(topic.id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "seed");

    let mut committed = manager.begin().await?;
    assert_eq!(
        topics
            .service
            .promote_enriched(committed.as_mut(), topic.id)
            .await?,
        TopicStatus::Enriched
    );
    assert_eq!(
        topics
            .service
            .promote_enriched(committed.as_mut(), topic.id)
            .await?,
        TopicStatus::Enriched
    );
    manager.commit(committed).await?;
    let status: String = sqlx::query_scalar("SELECT status FROM topics WHERE id = $1")
        .bind(topic.id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "enriched");

    pool.close().await;
    database.dispose().await
}
