use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use jamye_server::application::topics::{TopicDatePageInput, TopicPageInput, TopicsError};

use crate::{
    TestResult,
    postgres_support::TestDatabase,
    topic_helpers::{create_topic, harness, topology},
};

#[tokio::test]
async fn timeline_pages_are_strictly_descending_and_use_seoul_calendar_boundaries() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let topics = harness(pool.clone());
    let before_midnight = create_topic(
        &topics,
        fixture.author_id,
        fixture.group_id,
        Uuid::new_v4(),
        "서울 자정 전",
    )
    .await?;
    let at_midnight = create_topic(
        &topics,
        fixture.author_id,
        fixture.group_id,
        Uuid::new_v4(),
        "서울 자정",
    )
    .await?;
    let later = create_topic(
        &topics,
        fixture.author_id,
        fixture.group_id,
        Uuid::new_v4(),
        "서울 오전",
    )
    .await?;
    for (topic_id, created_at) in [
        (
            before_midnight.id,
            OffsetDateTime::parse("2025-12-31T14:59:59Z", &Rfc3339)?,
        ),
        (
            at_midnight.id,
            OffsetDateTime::parse("2025-12-31T15:00:00Z", &Rfc3339)?,
        ),
        (
            later.id,
            OffsetDateTime::parse("2026-01-01T00:00:00Z", &Rfc3339)?,
        ),
    ] {
        sqlx::query("UPDATE topics SET created_at = $2 WHERE id = $1")
            .bind(topic_id)
            .bind(created_at)
            .execute(&pool)
            .await?;
    }

    let first = topics
        .service
        .list_topics(
            fixture.member_id,
            fixture.group_id,
            TopicPageInput {
                after: None,
                limit: Some(1),
                date: None,
            },
        )
        .await?;
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.items[0].id, later.id);
    let cursor = first
        .next_cursor
        .ok_or_else(|| std::io::Error::other("first topic page omitted cursor"))?;
    let second = topics
        .service
        .list_topics(
            fixture.member_id,
            fixture.group_id,
            TopicPageInput {
                after: Some(cursor),
                limit: Some(2),
                date: None,
            },
        )
        .await?;
    assert_eq!(
        second
            .items
            .iter()
            .map(|topic| topic.id)
            .collect::<Vec<_>>(),
        vec![at_midnight.id, before_midnight.id]
    );
    assert!(second.next_cursor.is_none());

    let january = topics
        .service
        .list_topics(
            fixture.member_id,
            fixture.group_id,
            TopicPageInput {
                after: None,
                limit: Some(10),
                date: Some("2026-01-01".to_owned()),
            },
        )
        .await?;
    assert_eq!(
        january
            .items
            .iter()
            .map(|topic| topic.id)
            .collect::<Vec<_>>(),
        vec![later.id, at_midnight.id]
    );
    let december = topics
        .service
        .list_topics(
            fixture.member_id,
            fixture.group_id,
            TopicPageInput {
                after: None,
                limit: Some(10),
                date: Some("2025-12-31".to_owned()),
            },
        )
        .await?;
    assert_eq!(december.items.len(), 1);
    assert_eq!(december.items[0].id, before_midnight.id);

    let dates = topics
        .service
        .list_topic_dates(
            fixture.member_id,
            fixture.group_id,
            TopicDatePageInput {
                after: None,
                limit: Some(100),
            },
        )
        .await?;
    assert!(dates.dates.contains(&"2026-01-01".to_owned()));
    assert!(dates.dates.contains(&"2025-12-31".to_owned()));
    assert!(dates.dates.contains(&dates.today));
    assert!(dates.dates.windows(2).all(|window| window[0] > window[1]));

    for invalid in ["2026-02-30", "2026-1-01", "not-a-date", "가1-01-01"] {
        assert_eq!(
            topics
                .service
                .list_topics(
                    fixture.member_id,
                    fixture.group_id,
                    TopicPageInput {
                        after: None,
                        limit: None,
                        date: Some(invalid.to_owned()),
                    },
                )
                .await,
            Err(TopicsError::RequestValidation)
        );
    }

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn topic_queries_hide_soft_deleted_groups_and_enforce_membership() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let topics = harness(pool.clone());
    let topic = create_topic(
        &topics,
        fixture.author_id,
        fixture.group_id,
        Uuid::new_v4(),
        "접근 주제",
    )
    .await?;
    assert_eq!(
        topics
            .service
            .get_topic(fixture.outsider_id, fixture.group_id, topic.id)
            .await,
        Err(TopicsError::MembershipRequired)
    );
    sqlx::query("UPDATE groups SET deleted_at = clock_timestamp() WHERE id = $1")
        .bind(fixture.group_id)
        .execute(&pool)
        .await?;
    assert_eq!(
        topics
            .service
            .get_topic(fixture.author_id, fixture.group_id, topic.id)
            .await,
        Err(TopicsError::GroupNotFound)
    );
    assert_eq!(
        topics
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
            .await,
        Err(TopicsError::GroupNotFound)
    );

    pool.close().await;
    database.dispose().await
}
