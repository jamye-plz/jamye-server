use std::{fmt, sync::Arc, time::Duration};

use time::OffsetDateTime;
use uuid::Uuid;

use crate::ports::{
    auth::{
        AccessTokenIssuer, AuthClock, AuthRepository, CredentialSource, NewProviderIdentity,
        NewRefreshSession, NewRotatedSession, RotationOutcome,
    },
    oauth_attempt::{
        ConsumeAttemptOutcome, CreateAttemptOutcome, OAuthAttempt, OAuthAttemptError,
        OAuthAttemptStore,
    },
    oauth_provider::{
        AuthorizationRequest, OAuthProvider, OAuthProviderError, ProviderExchangeRequest,
        ProviderKind,
    },
    rate_limit::{RateLimitOutcome, RateLimitRequest, RateLimiter},
    transactions::{BoxTransactionHandle, TransactionManager},
};

pub const OAUTH_ATTEMPT_TTL: Duration = Duration::from_secs(600);
const MAX_CREDENTIAL_GENERATION_ATTEMPTS: usize = 3;

#[derive(Clone)]
pub struct AuthService {
    dependencies: AuthDependencies,
    kakao: OAuthProviderSlot,
    google: OAuthProviderSlot,
    lifetimes: AuthLifetimePolicy,
    rate_limits: AuthRateLimitPolicy,
}

#[derive(Clone)]
pub struct AuthDependencies {
    pub transactions: Arc<dyn TransactionManager>,
    pub repository: Arc<dyn AuthRepository>,
    pub attempts: Arc<dyn OAuthAttemptStore>,
    pub rate_limiter: Arc<dyn RateLimiter>,
    pub credentials: Arc<dyn CredentialSource>,
    pub token_issuer: Arc<dyn AccessTokenIssuer>,
    pub clock: Arc<dyn AuthClock>,
}

#[derive(Clone)]
pub struct OAuthProviderSlot {
    kind: ProviderKind,
    enabled: bool,
    redirect_uris: Vec<String>,
    provider: Option<Arc<dyn OAuthProvider>>,
}

impl OAuthProviderSlot {
    pub fn disabled(kind: ProviderKind) -> Self {
        Self {
            kind,
            enabled: false,
            redirect_uris: Vec::new(),
            provider: None,
        }
    }

    pub fn enabled(
        kind: ProviderKind,
        redirect_uris: Vec<String>,
        provider: Arc<dyn OAuthProvider>,
    ) -> Result<Self, AuthError> {
        if provider.kind() != kind
            || redirect_uris.is_empty()
            || redirect_uris
                .iter()
                .any(|uri| uri.is_empty() || uri.contains('*'))
        {
            return Err(AuthError::InvalidConfiguration);
        }
        Ok(Self {
            kind,
            enabled: true,
            redirect_uris,
            provider: Some(provider),
        })
    }

    fn allows_redirect(&self, redirect_uri: &str) -> bool {
        self.redirect_uris
            .iter()
            .any(|allowed| allowed == redirect_uri)
    }
}

impl AuthService {
    pub fn new(
        dependencies: AuthDependencies,
        kakao: OAuthProviderSlot,
        google: OAuthProviderSlot,
        lifetimes: AuthLifetimePolicy,
        rate_limits: AuthRateLimitPolicy,
    ) -> Result<Self, AuthError> {
        if kakao.kind != ProviderKind::Kakao
            || google.kind != ProviderKind::Google
            || lifetimes.access.is_zero()
            || lifetimes.refresh <= lifetimes.access
            || !rate_limits.is_valid()
        {
            return Err(AuthError::InvalidConfiguration);
        }
        Ok(Self {
            dependencies,
            kakao,
            google,
            lifetimes,
            rate_limits,
        })
    }

    pub async fn authorize(
        &self,
        provider_path: &str,
        input: AuthorizeInput,
        network_subject: &str,
    ) -> Result<AuthorizeOutput, AuthError> {
        let slot = self.provider_slot(provider_path)?;
        validate_authorize_input(&input)?;
        if !slot.allows_redirect(&input.redirect_uri) {
            return Err(AuthError::OAuthAuthorizeInvalid);
        }
        self.check_rate_limit(
            "oauth_authorize",
            network_subject,
            &self.rate_limits.authorize,
        )
        .await?;
        for _ in 0..MAX_CREDENTIAL_GENERATION_ATTEMPTS {
            let state = self
                .dependencies
                .credentials
                .generate()
                .map_err(|_| AuthError::OAuthCoordinationUnavailable)?;
            let nonce = self
                .dependencies
                .credentials
                .generate()
                .map_err(|_| AuthError::OAuthCoordinationUnavailable)?;
            let attempt = OAuthAttempt {
                provider: slot.kind,
                redirect_uri: input.redirect_uri.clone(),
                code_challenge: input.code_challenge.clone(),
                nonce: nonce.raw.expose().to_owned(),
            };
            match self
                .dependencies
                .attempts
                .create(&state.digest, &attempt, OAUTH_ATTEMPT_TTL)
                .await
                .map_err(map_attempt_error)?
            {
                CreateAttemptOutcome::Created => {
                    let authorization_url = slot
                        .provider
                        .as_ref()
                        .ok_or(AuthError::InvalidConfiguration)?
                        .authorization_url(&AuthorizationRequest {
                            redirect_uri: input.redirect_uri,
                            state: state.raw.expose().to_owned(),
                            code_challenge: input.code_challenge,
                            nonce: nonce.raw.into_string(),
                        })
                        .map_err(map_provider_error)?;
                    return Ok(AuthorizeOutput {
                        authorization_url,
                        state: state.raw.into_string(),
                        expires_in_seconds: OAUTH_ATTEMPT_TTL.as_secs(),
                    });
                }
                CreateAttemptOutcome::Collision => continue,
            }
        }
        Err(AuthError::OAuthCoordinationUnavailable)
    }

    pub async fn exchange(
        &self,
        provider_path: &str,
        input: ExchangeInput,
        network_subject: &str,
    ) -> Result<TokenPair, AuthError> {
        let slot = self.provider_slot(provider_path)?;
        validate_exchange_input(&input)?;
        if !slot.allows_redirect(&input.redirect_uri) {
            return Err(AuthError::OAuthExchangeInvalid);
        }
        self.check_rate_limit(
            "oauth_exchange",
            network_subject,
            &self.rate_limits.exchange,
        )
        .await?;
        let state_digest = self
            .dependencies
            .credentials
            .digest(&input.state)
            .map_err(|_| AuthError::OAuthExchangeInvalid)?;
        let attempt = match self
            .dependencies
            .attempts
            .consume(&state_digest)
            .await
            .map_err(map_attempt_error)?
        {
            ConsumeAttemptOutcome::Found(attempt) => attempt,
            ConsumeAttemptOutcome::Missing => return Err(AuthError::OAuthExchangeInvalid),
        };
        let actual_challenge = self
            .dependencies
            .credentials
            .pkce_s256(&input.code_verifier)
            .map_err(|_| AuthError::OAuthExchangeInvalid)?;
        if attempt.provider != slot.kind
            || attempt.redirect_uri != input.redirect_uri
            || attempt.code_challenge != actual_challenge
        {
            return Err(AuthError::OAuthExchangeInvalid);
        }
        let provider_identity = slot
            .provider
            .as_ref()
            .ok_or(AuthError::InvalidConfiguration)?
            .exchange(&ProviderExchangeRequest {
                authorization_code: input.authorization_code,
                redirect_uri: input.redirect_uri,
                code_verifier: input.code_verifier,
                nonce: attempt.nonce,
            })
            .await
            .map_err(map_provider_error)?;
        if !valid_provider_identity(&provider_identity) {
            return Err(AuthError::OAuthProviderUnavailable);
        }
        let refresh = self
            .dependencies
            .credentials
            .generate()
            .map_err(|_| AuthError::DatabaseUnavailable)?;
        let now = self.dependencies.clock.now();
        let access_expires_at = add_duration(now, self.lifetimes.access)?;
        let refresh_expires_at = add_duration(now, self.lifetimes.refresh)?;
        let session_id = Uuid::new_v4();
        let session = NewRefreshSession {
            id: session_id,
            family_id: Uuid::new_v4(),
            parent_session_id: None,
            token_hash: refresh.digest,
            expires_at: refresh_expires_at,
        };
        let identity = NewProviderIdentity {
            provider: slot.kind.as_str().to_owned(),
            provider_id: provider_identity.provider_id,
            nickname: provider_identity.nickname,
            avatar_url: provider_identity.avatar_url,
        };
        let mut transaction = self.begin().await?;
        let issued = match self
            .dependencies
            .repository
            .create_session(transaction.as_mut(), &identity, &session)
            .await
        {
            Ok(issued) => issued,
            Err(_) => {
                return self
                    .rollback_with(transaction, AuthError::DatabaseUnavailable)
                    .await;
            }
        };
        let token_pair = match self.issue_token_pair(
            issued.user_id,
            issued.session_id,
            now,
            access_expires_at,
            refresh.raw.into_string(),
            refresh_expires_at,
        ) {
            Ok(pair) => pair,
            Err(error) => return self.rollback_with(transaction, error).await,
        };
        self.commit(transaction).await?;
        Ok(token_pair)
    }

    pub async fn refresh(&self, raw_refresh_token: &str) -> Result<TokenPair, AuthError> {
        validate_refresh_token(raw_refresh_token)?;
        self.check_rate_limit("auth_refresh", raw_refresh_token, &self.rate_limits.refresh)
            .await?;
        let parent_digest = self
            .dependencies
            .credentials
            .digest(raw_refresh_token)
            .map_err(|_| AuthError::RequestValidation)?;
        let child_credential = self
            .dependencies
            .credentials
            .generate()
            .map_err(|_| AuthError::DatabaseUnavailable)?;
        let now = self.dependencies.clock.now();
        let access_expires_at = add_duration(now, self.lifetimes.access)?;
        let refresh_expires_at = add_duration(now, self.lifetimes.refresh)?;
        let child = NewRotatedSession {
            id: Uuid::new_v4(),
            token_hash: child_credential.digest,
            expires_at: refresh_expires_at,
        };
        let mut transaction = self.begin().await?;
        let outcome = match self
            .dependencies
            .repository
            .rotate_session(transaction.as_mut(), &parent_digest, &child, now)
            .await
        {
            Ok(outcome) => outcome,
            Err(_) => {
                return self
                    .rollback_with(transaction, AuthError::DatabaseUnavailable)
                    .await;
            }
        };
        match outcome {
            RotationOutcome::Rotated(issued) => {
                let token_pair = match self.issue_token_pair(
                    issued.user_id,
                    issued.session_id,
                    now,
                    access_expires_at,
                    child_credential.raw.into_string(),
                    refresh_expires_at,
                ) {
                    Ok(pair) => pair,
                    Err(error) => return self.rollback_with(transaction, error).await,
                };
                self.commit(transaction).await?;
                Ok(token_pair)
            }
            RotationOutcome::Invalid => {
                self.rollback_with(transaction, AuthError::RefreshTokenInvalid)
                    .await
            }
            RotationOutcome::Reused => {
                self.commit(transaction).await?;
                Err(AuthError::RefreshTokenReused)
            }
        }
    }

    pub async fn logout(&self, session_id: Uuid, principal_subject: &str) -> Result<(), AuthError> {
        self.check_rate_limit("auth_logout", principal_subject, &self.rate_limits.logout)
            .await?;
        let mut transaction = self.begin().await?;
        if self
            .dependencies
            .repository
            .revoke_session(
                transaction.as_mut(),
                session_id,
                self.dependencies.clock.now(),
            )
            .await
            .is_err()
        {
            return self
                .rollback_with(transaction, AuthError::DatabaseUnavailable)
                .await;
        }
        self.commit(transaction).await
    }

    fn provider_slot(&self, provider_path: &str) -> Result<&OAuthProviderSlot, AuthError> {
        let slot = match ProviderKind::parse(provider_path) {
            Some(ProviderKind::Kakao) => &self.kakao,
            Some(ProviderKind::Google) => &self.google,
            None => return Err(AuthError::OAuthProviderNotSupported),
        };
        if !slot.enabled {
            return Err(AuthError::OAuthProviderNotAvailable);
        }
        Ok(slot)
    }

    async fn check_rate_limit(
        &self,
        endpoint: &'static str,
        subject: &str,
        policy: &EndpointRateLimit,
    ) -> Result<(), AuthError> {
        match self
            .dependencies
            .rate_limiter
            .check(&RateLimitRequest {
                endpoint,
                subject: subject.to_owned(),
                limit: policy.limit,
                window: policy.window,
            })
            .await
            .map_err(|_| AuthError::RateLimitUnavailable)?
        {
            RateLimitOutcome::Allowed => Ok(()),
            RateLimitOutcome::Denied { retry_after } => Err(AuthError::RateLimited { retry_after }),
        }
    }

    fn issue_token_pair(
        &self,
        user_id: Uuid,
        session_id: Uuid,
        now: OffsetDateTime,
        access_expires_at: OffsetDateTime,
        refresh_token: String,
        refresh_expires_at: OffsetDateTime,
    ) -> Result<TokenPair, AuthError> {
        let access_token = self
            .dependencies
            .token_issuer
            .issue(user_id, session_id, now, access_expires_at)
            .map_err(|_| AuthError::TokenIssuanceUnavailable)?;
        Ok(TokenPair {
            token_type: "Bearer",
            access_token,
            access_token_expires_at: access_expires_at,
            refresh_token,
            refresh_token_expires_at: refresh_expires_at,
        })
    }

    async fn begin(&self) -> Result<BoxTransactionHandle, AuthError> {
        self.dependencies
            .transactions
            .begin()
            .await
            .map_err(|_| AuthError::DatabaseUnavailable)
    }

    async fn commit(&self, transaction: BoxTransactionHandle) -> Result<(), AuthError> {
        self.dependencies
            .transactions
            .commit(transaction)
            .await
            .map_err(|_| AuthError::DatabaseUnavailable)
    }

    async fn rollback_with<T>(
        &self,
        transaction: BoxTransactionHandle,
        error: AuthError,
    ) -> Result<T, AuthError> {
        self.dependencies
            .transactions
            .rollback(transaction)
            .await
            .map_err(|_| AuthError::DatabaseUnavailable)?;
        Err(error)
    }
}

fn validate_authorize_input(input: &AuthorizeInput) -> Result<(), AuthError> {
    if input.redirect_uri.is_empty() || !valid_base64url_credential(&input.code_challenge) {
        return Err(AuthError::RequestValidation);
    }
    if input.code_challenge_method != "S256" {
        return Err(AuthError::OAuthAuthorizeInvalid);
    }
    Ok(())
}

fn validate_exchange_input(input: &ExchangeInput) -> Result<(), AuthError> {
    if input.authorization_code.is_empty()
        || input.authorization_code.len() > 4096
        || !valid_base64url_credential(&input.state)
        || !valid_pkce_verifier(&input.code_verifier)
        || input.redirect_uri.is_empty()
    {
        return Err(AuthError::RequestValidation);
    }
    Ok(())
}

fn validate_refresh_token(token: &str) -> Result<(), AuthError> {
    if valid_base64url_credential(token) {
        Ok(())
    } else {
        Err(AuthError::RequestValidation)
    }
}

fn valid_provider_identity(identity: &crate::ports::oauth_provider::ProviderIdentity) -> bool {
    !identity.provider_id.is_empty()
        && identity.provider_id.chars().count() <= 128
        && !identity.nickname.is_empty()
        && identity.nickname.chars().count() <= 64
        && identity
            .avatar_url
            .as_ref()
            .is_none_or(|avatar_url| avatar_url.chars().count() <= 512)
}

fn valid_base64url_credential(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn valid_pkce_verifier(value: &str) -> bool {
    (43..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
}

fn map_attempt_error(error: OAuthAttemptError) -> AuthError {
    match error {
        OAuthAttemptError::Unavailable => AuthError::OAuthCoordinationUnavailable,
        OAuthAttemptError::InvalidData => AuthError::OAuthExchangeInvalid,
    }
}

fn map_provider_error(error: OAuthProviderError) -> AuthError {
    match error {
        OAuthProviderError::Unavailable | OAuthProviderError::InvalidIdentity => {
            AuthError::OAuthProviderUnavailable
        }
        OAuthProviderError::InvalidConfiguration => AuthError::InvalidConfiguration,
    }
}

fn add_duration(now: OffsetDateTime, duration: Duration) -> Result<OffsetDateTime, AuthError> {
    let seconds = i64::try_from(duration.as_secs()).map_err(|_| AuthError::InvalidConfiguration)?;
    now.checked_add(time::Duration::seconds(seconds))
        .ok_or(AuthError::InvalidConfiguration)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizeInput {
    pub redirect_uri: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizeOutput {
    pub authorization_url: String,
    pub state: String,
    pub expires_in_seconds: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExchangeInput {
    pub authorization_code: String,
    pub state: String,
    pub code_verifier: String,
    pub redirect_uri: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenPair {
    pub token_type: &'static str,
    pub access_token: String,
    pub access_token_expires_at: OffsetDateTime,
    pub refresh_token: String,
    pub refresh_token_expires_at: OffsetDateTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EndpointRateLimit {
    pub limit: u32,
    pub window: Duration,
}

impl EndpointRateLimit {
    fn is_valid(&self) -> bool {
        self.limit > 0 && !self.window.is_zero() && self.window <= Duration::from_secs(86_400)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthRateLimitPolicy {
    pub authorize: EndpointRateLimit,
    pub exchange: EndpointRateLimit,
    pub refresh: EndpointRateLimit,
    pub logout: EndpointRateLimit,
}

impl AuthRateLimitPolicy {
    fn is_valid(&self) -> bool {
        self.authorize.is_valid()
            && self.exchange.is_valid()
            && self.refresh.is_valid()
            && self.logout.is_valid()
    }
}

pub use crate::ports::auth::AuthLifetimePolicy;

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemAuthClock;

impl AuthClock for SystemAuthClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthError {
    RequestValidation,
    OAuthProviderNotSupported,
    OAuthProviderNotAvailable,
    OAuthAuthorizeInvalid,
    OAuthExchangeInvalid,
    OAuthCoordinationUnavailable,
    OAuthProviderUnavailable,
    RateLimited { retry_after: Duration },
    RateLimitUnavailable,
    RefreshTokenInvalid,
    RefreshTokenReused,
    DatabaseUnavailable,
    TokenIssuanceUnavailable,
    InvalidConfiguration,
}

impl fmt::Display for AuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("authentication operation failed")
    }
}

impl std::error::Error for AuthError {}
