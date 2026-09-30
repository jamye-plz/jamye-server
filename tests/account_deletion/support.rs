use std::{
    fmt::Debug,
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

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
    ports::{
        account_deletion::{
            AccountDeletionIdentity, AccountDeletionPreparation, AccountDeletionReport,
            AccountDeletionRepository, AccountDeletionRepositoryError,
            AccountDeletionRepositoryFuture,
        },
        apple_identity_provider::{
            AppleAuthorizationCodeRevocationRequest, AppleIdentity, AppleIdentityProvider,
            AppleIdentityProviderError, AppleIdentityProviderFuture,
            AppleIdentityVerificationRequest, AppleRevocationProvider,
            AppleRevocationProviderError, AppleRevocationProviderFuture,
        },
        rate_limit::{RateLimitFuture, RateLimitOutcome, RateLimitRequest, RateLimiter},
        transactions::TransactionHandle,
    },
    transport::http::{
        account_deletion::{AccountDeletionHttpState, router as account_deletion_router},
        users::{UserHttpState, router as user_router},
    },
};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

pub(super) fn test_router(pool: PgPool) -> TestResult<Router> {
    test_router_with_apple(pool, None, None)
}

pub(super) fn test_router_with_apple(
    pool: PgPool,
    apple_identity_provider: Option<Arc<FakeAppleIdentityProvider>>,
    apple_revocation_provider: Option<Arc<FakeAppleRevocationProvider>>,
) -> TestResult<Router> {
    test_router_with_apple_repository(
        pool.clone(),
        apple_identity_provider,
        apple_revocation_provider,
        Arc::new(PostgresAccountDeletionRepository::new(pool)),
    )
}

pub(super) fn test_router_with_apple_repository(
    pool: PgPool,
    apple_identity_provider: Option<Arc<FakeAppleIdentityProvider>>,
    apple_revocation_provider: Option<Arc<FakeAppleRevocationProvider>>,
    repository: Arc<dyn AccountDeletionRepository>,
) -> TestResult<Router> {
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
        repository,
        apple_identity_provider: apple_identity_provider.map(|provider| {
            let provider: Arc<dyn AppleIdentityProvider> = provider;
            provider
        }),
        apple_revocation_provider: apple_revocation_provider.map(|provider| {
            let provider: Arc<dyn AppleRevocationProvider> = provider;
            provider
        }),
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

pub(super) struct FailGraceAfterPrepareRepository {
    inner: Arc<PostgresAccountDeletionRepository>,
}

impl FailGraceAfterPrepareRepository {
    pub(super) fn new(pool: PgPool) -> Self {
        Self {
            inner: Arc::new(PostgresAccountDeletionRepository::new(pool)),
        }
    }
}

impl AccountDeletionRepository for FailGraceAfterPrepareRepository {
    fn live_identity(
        &self,
        user_id: Uuid,
    ) -> AccountDeletionRepositoryFuture<'_, Option<AccountDeletionIdentity>> {
        self.inner.live_identity(user_id)
    }

    fn prepare_deletion<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        user_id: Uuid,
    ) -> AccountDeletionRepositoryFuture<'a, AccountDeletionPreparation> {
        self.inner.prepare_deletion(transaction, user_id)
    }

    fn finalize_deletion<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        user_id: Uuid,
    ) -> AccountDeletionRepositoryFuture<'a, AccountDeletionReport> {
        self.inner.finalize_deletion(transaction, user_id)
    }

    fn start_grace_period<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _user_id: Uuid,
    ) -> AccountDeletionRepositoryFuture<'a, AccountDeletionReport> {
        Box::pin(async { Err(AccountDeletionRepositoryError::Unavailable) })
    }
}

pub(super) struct FakeAppleIdentityProvider {
    identity: AppleIdentity,
    error: Option<AppleIdentityProviderError>,
    calls: AtomicUsize,
}

impl FakeAppleIdentityProvider {
    pub(super) fn new(identity: AppleIdentity, error: Option<AppleIdentityProviderError>) -> Self {
        Self {
            identity,
            error,
            calls: AtomicUsize::new(0),
        }
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl AppleIdentityProvider for FakeAppleIdentityProvider {
    fn verify_identity<'a>(
        &'a self,
        _request: &'a AppleIdentityVerificationRequest,
    ) -> AppleIdentityProviderFuture<'a, AppleIdentity> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(error) = self.error {
                return Err(error);
            }
            Ok(self.identity.clone())
        })
    }
}

pub(super) struct FakeAppleRevocationProvider {
    error: Option<AppleRevocationProviderError>,
    calls: AtomicUsize,
    requests: Mutex<Vec<AppleAuthorizationCodeRevocationRequest>>,
}

impl FakeAppleRevocationProvider {
    pub(super) fn new(error: Option<AppleRevocationProviderError>) -> Self {
        Self {
            error,
            calls: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub(super) fn requests(&self) -> TestResult<Vec<AppleAuthorizationCodeRevocationRequest>> {
        Ok(self
            .requests
            .lock()
            .map_err(|_| io::Error::other("Apple revocation request lock poisoned"))?
            .clone())
    }
}

impl AppleRevocationProvider for FakeAppleRevocationProvider {
    fn revoke_authorization_code<'a>(
        &'a self,
        request: &'a AppleAuthorizationCodeRevocationRequest,
    ) -> AppleRevocationProviderFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.requests
                .lock()
                .map_err(|_| AppleRevocationProviderError::Unavailable)?
                .push(request.clone());
            if let Some(error) = self.error {
                return Err(error);
            }
            Ok(())
        })
    }
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
