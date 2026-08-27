use std::{sync::Arc, time::Duration};

use jamye_server::{
    adapters::{
        oauth::OsCredentialSource,
        postgres::{
            groups::PostgresGroupsRepository, push::PostgresPushRepository,
            realtime_revocations::PostgresRealtimeRevocations,
            transactions::SqlxTransactionManager,
        },
    },
    application::{
        groups::{
            GroupsDependencies, GroupsEndpointRateLimit, GroupsRateLimitPolicy, GroupsService,
            SystemGroupsClock,
        },
        realtime::membership_revocation::MembershipRevocationService,
    },
    ports::{
        push::PushPrivacyFence,
        rate_limit::{RateLimitFuture, RateLimitOutcome, RateLimitRequest, RateLimiter},
    },
};
use sqlx::PgPool;
use uuid::Uuid;

use crate::TestResult;

pub(super) fn membership_revocations(pool: PgPool) -> TestResult<MembershipRevocationService> {
    let privacy = PostgresPushRepository::new(pool.clone());
    membership_revocations_with_privacy(pool, privacy)
}

pub(super) fn membership_revocations_with_privacy(
    pool: PgPool,
    privacy: impl PushPrivacyFence + 'static,
) -> TestResult<MembershipRevocationService> {
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
    Ok(MembershipRevocationService::new(
        groups,
        transactions,
        Arc::new(PostgresRealtimeRevocations::new(pool.clone())),
        Arc::new(privacy),
    ))
}

pub(super) async fn assert_occurrence_fenced(pool: &PgPool, id: Uuid) -> TestResult {
    let state = sqlx::query_as::<_, (String, Option<String>, bool, Option<String>, bool)>(
        "SELECT status, claim_owner, lease_expires_at IS NULL, last_error_code, \
                failed_at IS NOT NULL \
         FROM push_delivery_intents WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    assert_eq!(
        state,
        (
            "failed".to_owned(),
            None,
            true,
            Some("privacy_revoked".to_owned()),
            true,
        )
    );
    Ok(())
}

#[derive(Clone, Copy)]
struct AllowRateLimiter;

impl RateLimiter for AllowRateLimiter {
    fn check<'a>(&'a self, _request: &'a RateLimitRequest) -> RateLimitFuture<'a> {
        Box::pin(async { Ok(RateLimitOutcome::Allowed) })
    }
}
