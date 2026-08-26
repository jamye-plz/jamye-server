use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use jamye_server::{
    adapters::{
        oauth::{OsCredentialSource, ProductionTokenCodec},
        postgres::{auth::PostgresAuthRepository, transactions::SqlxTransactionManager},
    },
    application::auth::{
        AuthDependencies, AuthLifetimePolicy, AuthRateLimitPolicy, AuthService, AuthorizeInput,
        EndpointRateLimit, ExchangeInput, OAuthProviderSlot, TokenPair,
    },
    ports::{
        auth::{AuthClock, CredentialDigest, CredentialSource},
        oauth_attempt::{
            ConsumeAttemptOutcome, CreateAttemptOutcome, OAuthAttempt, OAuthAttemptError,
            OAuthAttemptFuture, OAuthAttemptStore,
        },
        oauth_provider::{
            AuthorizationRequest, OAuthProvider, OAuthProviderError, OAuthProviderFuture,
            ProviderExchangeRequest, ProviderIdentity, ProviderKind,
        },
        rate_limit::{RateLimitFuture, RateLimitOutcome, RateLimitRequest, RateLimiter},
    },
};
use sqlx::PgPool;
use time::OffsetDateTime;
use tokio::sync::Barrier;

use crate::TestResult;

pub const KAKAO_REDIRECT: &str = "jamye://oauth/kakao";
pub const GOOGLE_REDIRECT: &str = "jamye://oauth/google";
pub const TEST_VERIFIER: &str =
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~";

pub struct TestAuthHarness {
    pub service: Arc<AuthService>,
    pub codec: Arc<ProductionTokenCodec>,
    pub attempts: Arc<MemoryAttemptStore>,
    pub provider: Arc<FakeProvider>,
}

pub fn harness(
    pool: PgPool,
    provider_barrier: Option<Arc<Barrier>>,
) -> TestResult<TestAuthHarness> {
    harness_with_options(
        pool,
        provider_barrier,
        Arc::new(AllowRateLimiter),
        None,
        None,
    )
}

pub fn harness_with_rate_limiter(
    pool: PgPool,
    provider_barrier: Option<Arc<Barrier>>,
    rate_limiter: Arc<dyn RateLimiter>,
) -> TestResult<TestAuthHarness> {
    harness_with_options(pool, provider_barrier, rate_limiter, None, None)
}

pub fn harness_with_provider_error(
    pool: PgPool,
    error: OAuthProviderError,
) -> TestResult<TestAuthHarness> {
    harness_with_options(pool, None, Arc::new(AllowRateLimiter), Some(error), None)
}

pub fn harness_with_provider_identity(
    pool: PgPool,
    identity: ProviderIdentity,
) -> TestResult<TestAuthHarness> {
    harness_with_options(pool, None, Arc::new(AllowRateLimiter), None, Some(identity))
}

fn harness_with_options(
    pool: PgPool,
    provider_barrier: Option<Arc<Barrier>>,
    rate_limiter: Arc<dyn RateLimiter>,
    provider_error: Option<OAuthProviderError>,
    provider_identity: Option<ProviderIdentity>,
) -> TestResult<TestAuthHarness> {
    let repository = Arc::new(PostgresAuthRepository::new(pool.clone()));
    let transactions = Arc::new(SqlxTransactionManager::new(pool));
    let attempts = Arc::new(MemoryAttemptStore::default());
    let provider = Arc::new(FakeProvider::new(
        ProviderKind::Kakao,
        provider_identity.unwrap_or_else(|| ProviderIdentity {
            provider_id: "kakao-principal-42".to_owned(),
            nickname: "잠이 사용자".to_owned(),
            avatar_url: Some("https://images.example/avatar.png".to_owned()),
        }),
        provider_barrier,
        provider_error,
    ));
    let codec = Arc::new(ProductionTokenCodec::new(
        b"task-5-test-signing-secret-32-bytes-minimum",
        "https://api.jamye.test",
        "jamye-mobile",
    )?);
    let one_minute = Duration::from_secs(60);
    let service = AuthService::new(
        AuthDependencies {
            transactions,
            repository,
            attempts: attempts.clone(),
            rate_limiter,
            credentials: Arc::new(OsCredentialSource),
            token_issuer: codec.clone(),
            clock: Arc::new(FixedClock(OffsetDateTime::now_utc())),
        },
        OAuthProviderSlot::enabled(
            ProviderKind::Kakao,
            vec![KAKAO_REDIRECT.to_owned()],
            provider.clone(),
        )?,
        OAuthProviderSlot::disabled(ProviderKind::Google),
        AuthLifetimePolicy {
            access: Duration::from_secs(900),
            refresh: Duration::from_secs(2_592_000),
        },
        AuthRateLimitPolicy {
            authorize: EndpointRateLimit {
                limit: 10,
                window: one_minute,
            },
            exchange: EndpointRateLimit {
                limit: 10,
                window: one_minute,
            },
            refresh: EndpointRateLimit {
                limit: 10,
                window: one_minute,
            },
            logout: EndpointRateLimit {
                limit: 10,
                window: one_minute,
            },
        },
    )?;
    Ok(TestAuthHarness {
        service: Arc::new(service),
        codec,
        attempts,
        provider,
    })
}

pub async fn authorize(service: &AuthService) -> TestResult<String> {
    let code_challenge = OsCredentialSource.pkce_s256(TEST_VERIFIER)?;
    Ok(service
        .authorize(
            "kakao",
            AuthorizeInput {
                redirect_uri: KAKAO_REDIRECT.to_owned(),
                code_challenge,
                code_challenge_method: "S256".to_owned(),
            },
            "ip:127.0.0.1",
        )
        .await?
        .state)
}

pub async fn exchange(
    service: &AuthService,
    state: String,
) -> Result<TokenPair, jamye_server::application::auth::AuthError> {
    service
        .exchange(
            "kakao",
            ExchangeInput {
                authorization_code: "provider-code".to_owned(),
                state,
                code_verifier: TEST_VERIFIER.to_owned(),
                redirect_uri: KAKAO_REDIRECT.to_owned(),
            },
            "ip:127.0.0.1",
        )
        .await
}

#[derive(Default)]
pub struct MemoryAttemptStore {
    attempts: Mutex<Vec<(CredentialDigest, OAuthAttempt)>>,
}

impl MemoryAttemptStore {
    pub fn len(&self) -> TestResult<usize> {
        Ok(self
            .attempts
            .lock()
            .map_err(|_| "OAuth attempt fixture lock poisoned")?
            .len())
    }
}

impl OAuthAttemptStore for MemoryAttemptStore {
    fn create<'a>(
        &'a self,
        state_digest: &'a CredentialDigest,
        attempt: &'a OAuthAttempt,
        _ttl: Duration,
    ) -> OAuthAttemptFuture<'a, CreateAttemptOutcome> {
        Box::pin(async move {
            let mut attempts = self
                .attempts
                .lock()
                .map_err(|_| OAuthAttemptError::Unavailable)?;
            if attempts.iter().any(|(digest, _)| digest == state_digest) {
                return Ok(CreateAttemptOutcome::Collision);
            }
            attempts.push((state_digest.clone(), attempt.clone()));
            Ok(CreateAttemptOutcome::Created)
        })
    }

    fn consume<'a>(
        &'a self,
        state_digest: &'a CredentialDigest,
    ) -> OAuthAttemptFuture<'a, ConsumeAttemptOutcome> {
        Box::pin(async move {
            let mut attempts = self
                .attempts
                .lock()
                .map_err(|_| OAuthAttemptError::Unavailable)?;
            let Some(index) = attempts
                .iter()
                .position(|(digest, _)| digest == state_digest)
            else {
                return Ok(ConsumeAttemptOutcome::Missing);
            };
            Ok(ConsumeAttemptOutcome::Found(attempts.swap_remove(index).1))
        })
    }
}

pub struct FakeProvider {
    kind: ProviderKind,
    identity: ProviderIdentity,
    barrier: Option<Arc<Barrier>>,
    error: Option<OAuthProviderError>,
    exchange_calls: AtomicUsize,
}

impl FakeProvider {
    fn new(
        kind: ProviderKind,
        identity: ProviderIdentity,
        barrier: Option<Arc<Barrier>>,
        error: Option<OAuthProviderError>,
    ) -> Self {
        Self {
            kind,
            identity,
            barrier,
            error,
            exchange_calls: AtomicUsize::new(0),
        }
    }

    pub fn exchange_calls(&self) -> usize {
        self.exchange_calls.load(Ordering::SeqCst)
    }
}

impl OAuthProvider for FakeProvider {
    fn kind(&self) -> ProviderKind {
        self.kind
    }

    fn authorization_url(
        &self,
        request: &AuthorizationRequest,
    ) -> Result<String, OAuthProviderError> {
        Ok(format!(
            "https://provider.example/authorize?state={}&code_challenge={}",
            request.state, request.code_challenge
        ))
    }

    fn exchange<'a>(
        &'a self,
        _request: &'a ProviderExchangeRequest,
    ) -> OAuthProviderFuture<'a, ProviderIdentity> {
        Box::pin(async move {
            self.exchange_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(barrier) = &self.barrier {
                barrier.wait().await;
            }
            if let Some(error) = self.error {
                return Err(error);
            }
            Ok(self.identity.clone())
        })
    }
}

struct AllowRateLimiter;

impl RateLimiter for AllowRateLimiter {
    fn check<'a>(&'a self, _request: &'a RateLimitRequest) -> RateLimitFuture<'a> {
        Box::pin(async { Ok(RateLimitOutcome::Allowed) })
    }
}

struct FixedClock(OffsetDateTime);

impl AuthClock for FixedClock {
    fn now(&self) -> OffsetDateTime {
        self.0
    }
}
