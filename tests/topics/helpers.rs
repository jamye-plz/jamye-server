use std::{sync::Arc, time::Duration};

use jamye_server::{
    adapters::{
        oauth::OsCredentialSource,
        postgres::{
            groups::PostgresGroupsRepository, topics::PostgresTopicsRepository,
            transactions::SqlxTransactionManager,
        },
    },
    application::{
        auth::{AccessIdentity, AccessTokenVerifier, AuthenticationError},
        groups::{
            GroupsDependencies, GroupsEndpointRateLimit, GroupsRateLimitPolicy, GroupsService,
        },
        topics::{TopicCreateInput, TopicsDependencies, TopicsService},
    },
    ports::{
        groups::GroupsClock,
        rate_limit::{RateLimitFuture, RateLimitOutcome, RateLimitRequest, RateLimiter},
        topics::{CreateTopicOutcome, TopicRecord},
    },
};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::TestResult;

pub struct TopicsHarness {
    pub service: Arc<TopicsService>,
}

pub struct TopicTopology {
    pub owner_id: Uuid,
    pub author_id: Uuid,
    pub member_id: Uuid,
    pub outsider_id: Uuid,
    pub group_id: Uuid,
    pub main_chatroom_id: Uuid,
}

pub fn harness(pool: PgPool) -> TopicsHarness {
    let repository = Arc::new(PostgresTopicsRepository::new(pool.clone()));
    let transactions = Arc::new(SqlxTransactionManager::new(pool));
    TopicsHarness {
        service: Arc::new(TopicsService::new(TopicsDependencies {
            transactions,
            repository,
        })),
    }
}

pub fn groups_service(pool: PgPool) -> TestResult<Arc<GroupsService>> {
    let one_minute = Duration::from_secs(60);
    let service = GroupsService::new(
        GroupsDependencies {
            transactions: Arc::new(SqlxTransactionManager::new(pool.clone())),
            repository: Arc::new(PostgresGroupsRepository::new(pool)),
            rate_limiter: Arc::new(AllowRateLimiter),
            credentials: Arc::new(OsCredentialSource),
            clock: Arc::new(FixedGroupsClock),
        },
        GroupsRateLimitPolicy {
            invite_issue: GroupsEndpointRateLimit {
                limit: 1,
                window: one_minute,
            },
            invite_redeem: GroupsEndpointRateLimit {
                limit: 1,
                window: one_minute,
            },
        },
    )?;
    Ok(Arc::new(service))
}

pub async fn topology(pool: &PgPool) -> TestResult<TopicTopology> {
    let owner_id = insert_user(pool, "주제 소유자", Some("https://cdn.test/owner.png")).await?;
    let author_id = insert_user(pool, "주제 작성자", Some("https://cdn.test/author.png")).await?;
    let member_id = insert_user(pool, "주제 멤버", None).await?;
    let outsider_id = insert_user(pool, "주제 외부인", None).await?;
    let group_id = Uuid::new_v4();
    sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, $2, $3)")
        .bind(group_id)
        .bind("주제 테스트 그룹")
        .bind(owner_id)
        .execute(pool)
        .await?;
    for (user_id, role) in [
        (owner_id, "owner"),
        (author_id, "member"),
        (member_id, "member"),
    ] {
        sqlx::query(
            "INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, $4)",
        )
        .bind(Uuid::new_v4())
        .bind(group_id)
        .bind(user_id)
        .bind(role)
        .execute(pool)
        .await?;
    }
    let main_chatroom_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO chatrooms (id, group_id, type, topic_id) VALUES ($1, $2, 'main', NULL)",
    )
    .bind(main_chatroom_id)
    .bind(group_id)
    .execute(pool)
    .await?;
    Ok(TopicTopology {
        owner_id,
        author_id,
        member_id,
        outsider_id,
        group_id,
        main_chatroom_id,
    })
}

pub async fn create_topic(
    harness: &TopicsHarness,
    author_id: Uuid,
    group_id: Uuid,
    key: Uuid,
    title: &str,
) -> TestResult<TopicRecord> {
    let outcome = harness
        .service
        .create_topic(
            author_id,
            group_id,
            TopicCreateInput {
                idempotency_key: key,
                title: title.to_owned(),
            },
        )
        .await?;
    match outcome {
        CreateTopicOutcome::Created(topic) | CreateTopicOutcome::Existing(topic) => Ok(topic),
    }
}

pub async fn insert_user(
    pool: &PgPool,
    nickname: &str,
    avatar_url: Option<&str>,
) -> TestResult<Uuid> {
    let user_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, nickname, avatar_url) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(nickname)
        .bind(avatar_url)
        .execute(pool)
        .await?;
    Ok(user_id)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TestAccessVerifier;

impl AccessTokenVerifier for TestAccessVerifier {
    fn verify(&self, token: &str) -> Result<AccessIdentity, AuthenticationError> {
        let user_id = token
            .strip_prefix("task7-")
            .and_then(|value| Uuid::try_parse(value).ok())
            .ok_or(AuthenticationError)?;
        Ok(AccessIdentity::new(user_id, Uuid::nil(), "task-7-test"))
    }
}

pub fn bearer(user_id: Uuid) -> String {
    format!("Bearer task7-{user_id}")
}

#[derive(Clone, Copy, Debug, Default)]
struct AllowRateLimiter;

impl RateLimiter for AllowRateLimiter {
    fn check<'a>(&'a self, _request: &'a RateLimitRequest) -> RateLimitFuture<'a> {
        Box::pin(async { Ok(RateLimitOutcome::Allowed) })
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct FixedGroupsClock;

impl GroupsClock for FixedGroupsClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::UNIX_EPOCH
    }
}
