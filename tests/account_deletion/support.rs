use std::{fmt::Debug, io, sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    http::{Request, header::AUTHORIZATION},
};
use jamye_server::{
    adapters::{
        oauth::OsCredentialSource,
        postgres::{
            account_deletion::PostgresAccountDeletionRepository, auth::PostgresAuthRepository,
            groups::PostgresGroupsRepository, push::PostgresPushRepository,
            transactions::SqlxTransactionManager,
        },
    },
    application::{
        account_deletion::{AccountDeletionDependencies, AccountDeletionService},
        auth::{AccessIdentity, AccessTokenVerifier, AuthenticationError},
        groups::{
            GroupsDependencies, GroupsEndpointRateLimit, GroupsRateLimitPolicy, GroupsService,
            SystemGroupsClock,
        },
        users::UserService,
    },
    ports::rate_limit::{RateLimitFuture, RateLimitOutcome, RateLimitRequest, RateLimiter},
    transport::http::{
        account_deletion::{AccountDeletionHttpState, router as account_deletion_router},
        users::{UserHttpState, router as user_router},
    },
};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

pub(super) fn test_router(pool: PgPool) -> TestResult<Router> {
    let verifier: Arc<dyn AccessTokenVerifier> = Arc::new(TestAccessVerifier);
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
    let account_deletion = Arc::new(AccountDeletionService::new(AccountDeletionDependencies {
        transactions: transactions.clone(),
        groups,
        push_privacy_fence: Arc::new(PostgresPushRepository::new(pool.clone())),
        repository: Arc::new(PostgresAccountDeletionRepository::new(pool.clone())),
    }));
    let users = Arc::new(UserService::new(
        transactions,
        Arc::new(PostgresAuthRepository::new(pool)),
    ));

    Ok(account_deletion_router(AccountDeletionHttpState::new(
        account_deletion,
        verifier.clone(),
    ))
    .merge(user_router(UserHttpState::new(users, verifier))))
}

pub(super) fn delete_request(user_id: Uuid) -> TestResult<Request<Body>> {
    authenticated_request("DELETE", "/api/v1/me", user_id, Body::empty())
}

pub(super) fn authenticated_request(
    method: &str,
    uri: &str,
    user_id: Uuid,
    body: Body,
) -> TestResult<Request<Body>> {
    Ok(Request::builder()
        .method(method)
        .uri(uri)
        .header(AUTHORIZATION, bearer(user_id))
        .body(body)?)
}

pub(super) fn bearer(user_id: Uuid) -> String {
    format!("Bearer task11-{user_id}")
}

pub(super) fn test_error(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(io::Error::other(message.into()))
}

pub(super) fn require(condition: bool, message: &str) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(test_error(message))
    }
}

pub(super) fn require_eq<T>(actual: T, expected: T, message: &str) -> TestResult
where
    T: Debug + PartialEq,
{
    if actual == expected {
        Ok(())
    } else {
        Err(test_error(format!(
            "{message}: actual={actual:?}, expected={expected:?}"
        )))
    }
}

pub(super) async fn finish_database_test(
    database: TestDatabase,
    pool: PgPool,
    result: TestResult,
) -> TestResult {
    pool.close().await;
    match (result, database.dispose().await) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Err(test_failure), Err(cleanup_failure)) => Err(test_error(format!(
            "test failed: {test_failure}; disposable database cleanup also failed: {cleanup_failure}"
        ))),
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct TestAccessVerifier;

impl AccessTokenVerifier for TestAccessVerifier {
    fn verify(&self, token: &str) -> Result<AccessIdentity, AuthenticationError> {
        let user_id = token
            .strip_prefix("task11-")
            .and_then(|value| Uuid::try_parse(value).ok())
            .ok_or(AuthenticationError)?;
        Ok(AccessIdentity::new(user_id, Uuid::nil(), "task-11-test"))
    }
}

#[derive(Clone, Copy)]
struct AllowRateLimiter;

impl RateLimiter for AllowRateLimiter {
    fn check<'a>(&'a self, _request: &'a RateLimitRequest) -> RateLimitFuture<'a> {
        Box::pin(async { Ok(RateLimitOutcome::Allowed) })
    }
}
