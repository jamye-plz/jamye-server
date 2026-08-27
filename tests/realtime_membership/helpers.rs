use std::{env, io, sync::Arc, time::Duration};

use jamye_server::{
    adapters::{
        oauth::OsCredentialSource,
        postgres::{
            groups::PostgresGroupsRepository, push::PostgresPushRepository,
            realtime_revocations::PostgresRealtimeRevocations,
            transactions::SqlxTransactionManager,
        },
        redis::realtime_control::RealtimeControlWorkerConfig,
    },
    application::{
        groups::{
            GroupCreateInput, GroupsDependencies, GroupsEndpointRateLimit, GroupsRateLimitPolicy,
            GroupsService, SystemGroupsClock,
        },
        realtime::membership_revocation::MembershipRevocationService,
    },
    ports::{
        groups::{GroupRecord, GroupRole},
        rate_limit::{RateLimitFuture, RateLimitOutcome, RateLimitRequest, RateLimiter},
    },
};
use sqlx::PgPool;
use url::Url;
use uuid::Uuid;

use crate::TestResult;

pub struct RevocationHarness {
    pub groups: Arc<GroupsService>,
    pub revocations: Arc<MembershipRevocationService>,
    pub store: PostgresRealtimeRevocations,
}

pub fn harness(pool: PgPool) -> TestResult<RevocationHarness> {
    let transactions = Arc::new(SqlxTransactionManager::new(pool.clone()));
    let groups = Arc::new(GroupsService::new(
        GroupsDependencies {
            transactions: transactions.clone(),
            repository: Arc::new(PostgresGroupsRepository::new(pool.clone())),
            rate_limiter: Arc::new(AllowRateLimiter),
            credentials: Arc::new(OsCredentialSource),
            clock: Arc::new(SystemGroupsClock),
        },
        GroupsRateLimitPolicy {
            invite_issue: GroupsEndpointRateLimit {
                limit: 10,
                window: Duration::from_secs(60),
            },
            invite_redeem: GroupsEndpointRateLimit {
                limit: 20,
                window: Duration::from_secs(60),
            },
        },
    )?);
    let store = PostgresRealtimeRevocations::new(pool.clone());
    let revocations = Arc::new(MembershipRevocationService::new(
        groups.clone(),
        transactions,
        Arc::new(store.clone()),
        Arc::new(PostgresPushRepository::new(pool.clone())),
    ));
    Ok(RevocationHarness {
        groups,
        revocations,
        store,
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

pub async fn insert_member(pool: &PgPool, group_id: Uuid, user_id: Uuid) -> TestResult<Uuid> {
    let membership_id = Uuid::new_v4();
    sqlx::query("INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, $4)")
        .bind(membership_id)
        .bind(group_id)
        .bind(user_id)
        .bind(GroupRole::Member.as_str())
        .execute(pool)
        .await?;
    Ok(membership_id)
}

pub async fn create_group(
    harness: &RevocationHarness,
    owner_id: Uuid,
    name: &str,
) -> TestResult<GroupRecord> {
    Ok(harness
        .groups
        .create_group(
            owner_id,
            GroupCreateInput {
                name: name.to_owned(),
            },
        )
        .await?)
}

pub fn guarded_redis_url() -> TestResult<String> {
    if env::var("JAMYE_ENVIRONMENT").as_deref() != Ok("test") {
        return Err(io::Error::other("task-6c Redis tests require JAMYE_ENVIRONMENT=test").into());
    }
    let redis_url = env::var("REDIS_URL")
        .map_err(|_| io::Error::other("REDIS_URL is required for task-6c tests"))?;
    let parsed = Url::parse(&redis_url)?;
    if parsed.scheme() != "redis"
        || !matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
    {
        return Err(
            io::Error::other("task-6c Redis tests accept only a loopback redis:// URL").into(),
        );
    }
    Ok(redis_url)
}

pub fn worker_config() -> RealtimeControlWorkerConfig {
    RealtimeControlWorkerConfig {
        claim_owner: format!("task-6c-{}", Uuid::new_v4()),
        batch_size: 16,
        lease_duration: Duration::from_secs(5),
        publish_timeout: Duration::from_secs(1),
        lease_safety_margin: Duration::from_millis(250),
        retry_delay: Duration::from_millis(100),
        max_attempts: 4,
    }
}

#[derive(Clone, Copy)]
struct AllowRateLimiter;

impl RateLimiter for AllowRateLimiter {
    fn check<'a>(&'a self, _request: &'a RateLimitRequest) -> RateLimitFuture<'a> {
        Box::pin(async { Ok(RateLimitOutcome::Allowed) })
    }
}
