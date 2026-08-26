use std::{sync::Arc, time::Duration};

use jamye_server::{
    adapters::{
        oauth::OsCredentialSource,
        postgres::{groups::PostgresGroupsRepository, transactions::SqlxTransactionManager},
    },
    application::{
        auth::{AccessIdentity, AccessTokenVerifier, AuthenticationError},
        groups::{
            GroupCreateInput, GroupsDependencies, GroupsEndpointRateLimit, GroupsRateLimitPolicy,
            GroupsService, InviteCreateInput,
        },
    },
    ports::{
        groups::{GroupRecord, GroupRole, GroupsClock, InviteRecord},
        rate_limit::{
            RateLimitError, RateLimitFuture, RateLimitOutcome, RateLimitRequest, RateLimiter,
        },
    },
};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::TestResult;

pub const TEST_NOW_UNIX: i64 = 1_800_000_000;

pub struct GroupsHarness {
    pub service: Arc<GroupsService>,
}

pub fn harness(pool: PgPool) -> TestResult<GroupsHarness> {
    harness_with_limiter(pool, Arc::new(AllowRateLimiter))
}

pub fn harness_with_limiter(
    pool: PgPool,
    rate_limiter: Arc<dyn RateLimiter>,
) -> TestResult<GroupsHarness> {
    let repository = Arc::new(PostgresGroupsRepository::new(pool.clone()));
    let transactions = Arc::new(SqlxTransactionManager::new(pool.clone()));
    let one_minute = Duration::from_secs(60);
    let service = GroupsService::new(
        GroupsDependencies {
            transactions,
            repository,
            rate_limiter,
            credentials: Arc::new(OsCredentialSource),
            clock: Arc::new(FixedGroupsClock(test_now())),
        },
        GroupsRateLimitPolicy {
            invite_issue: GroupsEndpointRateLimit {
                limit: 10,
                window: one_minute,
            },
            invite_redeem: GroupsEndpointRateLimit {
                limit: 20,
                window: one_minute,
            },
        },
    )?;
    Ok(GroupsHarness {
        service: Arc::new(service),
    })
}

pub async fn insert_user(pool: &PgPool, nickname: &str) -> TestResult<Uuid> {
    let user_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, $2)")
        .bind(user_id)
        .bind(nickname)
        .execute(pool)
        .await?;
    Ok(user_id)
}

pub async fn insert_member(
    pool: &PgPool,
    group_id: Uuid,
    user_id: Uuid,
    role: GroupRole,
) -> TestResult<Uuid> {
    let membership_id = Uuid::new_v4();
    sqlx::query("INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, $4)")
        .bind(membership_id)
        .bind(group_id)
        .bind(user_id)
        .bind(role.as_str())
        .execute(pool)
        .await?;
    Ok(membership_id)
}

pub async fn create_group(harness: &GroupsHarness, owner_id: Uuid) -> TestResult<GroupRecord> {
    Ok(harness
        .service
        .create_group(
            owner_id,
            GroupCreateInput {
                name: "테스트 그룹".to_owned(),
            },
        )
        .await?)
}

pub async fn create_invite(
    harness: &GroupsHarness,
    owner_id: Uuid,
    group_id: Uuid,
    max_uses: Option<i32>,
) -> TestResult<InviteRecord> {
    Ok(harness
        .service
        .create_invite(
            owner_id,
            group_id,
            InviteCreateInput {
                expires_at: Some(test_now() + time::Duration::hours(1)),
                max_uses,
            },
            &format!("user:{owner_id}:ip:127.0.0.1"),
        )
        .await?)
}

pub fn test_now() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(TEST_NOW_UNIX).unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

#[derive(Clone, Copy)]
struct FixedGroupsClock(OffsetDateTime);

impl GroupsClock for FixedGroupsClock {
    fn now(&self) -> OffsetDateTime {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AllowRateLimiter;

impl RateLimiter for AllowRateLimiter {
    fn check<'a>(&'a self, _request: &'a RateLimitRequest) -> RateLimitFuture<'a> {
        Box::pin(async { Ok(RateLimitOutcome::Allowed) })
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UnavailableRateLimiter;

impl RateLimiter for UnavailableRateLimiter {
    fn check<'a>(&'a self, _request: &'a RateLimitRequest) -> RateLimitFuture<'a> {
        Box::pin(async { Err(RateLimitError) })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DenyRateLimiter(pub Duration);

impl RateLimiter for DenyRateLimiter {
    fn check<'a>(&'a self, _request: &'a RateLimitRequest) -> RateLimitFuture<'a> {
        let retry_after = self.0;
        Box::pin(async move { Ok(RateLimitOutcome::Denied { retry_after }) })
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TestAccessVerifier;

impl AccessTokenVerifier for TestAccessVerifier {
    fn verify(&self, token: &str) -> Result<AccessIdentity, AuthenticationError> {
        let user_id = token
            .strip_prefix("task6-")
            .and_then(|value| Uuid::try_parse(value).ok())
            .ok_or(AuthenticationError)?;
        Ok(AccessIdentity::new(user_id, Uuid::nil(), "task-6-test"))
    }
}

pub fn bearer(user_id: Uuid) -> String {
    format!("Bearer task6-{user_id}")
}
