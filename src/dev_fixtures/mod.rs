//! Explicitly guarded, non-production fixture identity and seed service.

use std::{env, error::Error, fmt, future::Future, pin::Pin, sync::Arc};

use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, SignedDuration};
use uuid::Uuid;

use crate::application::auth::{AccessIdentity, AccessTokenVerifier, AuthenticationError};

pub const ENABLE_ENV: &str = "JAMYE_ENABLE_DEV_FIXTURES";
pub const ISSUER: &str = "jamye-dev";
pub const AUDIENCE: &str = "jamye-api";
const ENVIRONMENT_ENV: &str = "JAMYE_ENVIRONMENT";
const ACCESS_TOKEN_TTL: SignedDuration = SignedDuration::minutes(5);
const MINIMUM_HMAC_KEY_BYTES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DevFixtureGuard(());

impl DevFixtureGuard {
    pub fn from_env() -> Result<Self, DevFixtureInitError> {
        let enabled = env::var(ENABLE_ENV).ok();
        let environment = env::var(ENVIRONMENT_ENV).ok();
        Self::from_values(enabled.as_deref(), environment.as_deref())
    }

    fn from_values(
        enabled: Option<&str>,
        environment: Option<&str>,
    ) -> Result<Self, DevFixtureInitError> {
        let is_non_production = matches!(environment, Some("development" | "test"));
        if enabled == Some("true") && is_non_production {
            Ok(Self(()))
        } else {
            Err(DevFixtureInitError::RuntimeGuard)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DevFixtureInitError {
    RuntimeGuard,
    SigningKey,
}

impl fmt::Display for DevFixtureInitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RuntimeGuard => formatter.write_str(
                "dev fixtures require JAMYE_ENABLE_DEV_FIXTURES=true in development or test",
            ),
            Self::SigningKey => formatter.write_str("dev fixture signing key is not valid"),
        }
    }
}

impl Error for DevFixtureInitError {}

#[derive(Clone)]
pub struct DevTokenCodec {
    secret: Arc<[u8]>,
}

impl DevTokenCodec {
    pub fn ephemeral(guard: DevFixtureGuard) -> Self {
        let mut secret = Vec::with_capacity(MINIMUM_HMAC_KEY_BYTES);
        secret.extend_from_slice(Uuid::new_v4().as_bytes());
        secret.extend_from_slice(Uuid::new_v4().as_bytes());
        Self::from_guarded_secret(guard, secret)
    }

    pub fn from_secret(
        guard: DevFixtureGuard,
        secret: impl AsRef<[u8]>,
    ) -> Result<Self, DevFixtureInitError> {
        let secret = secret.as_ref();
        if secret.len() < MINIMUM_HMAC_KEY_BYTES {
            return Err(DevFixtureInitError::SigningKey);
        }
        Ok(Self::from_guarded_secret(guard, secret.to_vec()))
    }

    fn from_guarded_secret(_guard: DevFixtureGuard, secret: Vec<u8>) -> Self {
        Self {
            secret: Arc::from(secret),
        }
    }

    fn issue(&self, user_id: Uuid, session_id: Uuid) -> Result<String, DevTokenError> {
        let expires_at = OffsetDateTime::now_utc()
            .checked_add(ACCESS_TOKEN_TTL)
            .ok_or(DevTokenError)?;
        let exp = u64::try_from(expires_at.unix_timestamp()).map_err(|_| DevTokenError)?;
        let claims = DevAccessClaims {
            sub: user_id.to_string(),
            sid: session_id.to_string(),
            iss: ISSUER.to_owned(),
            aud: AUDIENCE.to_owned(),
            exp,
        };
        encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(self.secret.as_ref()),
        )
        .map_err(|_| DevTokenError)
    }

    fn validation() -> Validation {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.leeway = 0;
        validation.set_audience(&[AUDIENCE]);
        validation.set_issuer(&[ISSUER]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation
    }
}

impl AccessTokenVerifier for DevTokenCodec {
    fn verify(&self, token: &str) -> Result<AccessIdentity, AuthenticationError> {
        let token = decode::<DevAccessClaims>(
            token,
            &DecodingKey::from_secret(self.secret.as_ref()),
            &Self::validation(),
        )
        .map_err(|_| AuthenticationError)?;
        let user_id = Uuid::try_parse(&token.claims.sub).map_err(|_| AuthenticationError)?;
        let session_id = Uuid::try_parse(&token.claims.sid).map_err(|_| AuthenticationError)?;
        Ok(AccessIdentity::new(user_id, session_id, token.claims.iss))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DevAccessClaims {
    pub sub: String,
    pub sid: String,
    pub iss: String,
    pub aud: String,
    pub exp: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DevTokenError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixtureIds {
    pub(crate) user_id: Uuid,
    pub(crate) group_id: Uuid,
    pub(crate) membership_id: Uuid,
    pub(crate) chatroom_id: Uuid,
}

impl FixtureIds {
    fn generate() -> Self {
        Self {
            user_id: Uuid::new_v4(),
            group_id: Uuid::new_v4(),
            membership_id: Uuid::new_v4(),
            chatroom_id: Uuid::new_v4(),
        }
    }
}

pub type FixtureSeedFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), FixtureStoreError>> + Send + 'a>>;

pub trait DevFixtureStore: Send + Sync {
    fn seed(&self, fixture: FixtureIds) -> FixtureSeedFuture<'_>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixtureStoreError;

impl fmt::Display for FixtureStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("dev fixture persistence is unavailable")
    }
}

impl Error for FixtureStoreError {}

#[derive(Clone)]
pub struct DevFixtureService {
    store: Arc<dyn DevFixtureStore>,
    codec: DevTokenCodec,
}

impl DevFixtureService {
    pub fn new(store: Arc<dyn DevFixtureStore>, codec: DevTokenCodec) -> Self {
        Self { store, codec }
    }

    pub fn verifier(&self) -> Arc<dyn AccessTokenVerifier> {
        Arc::new(self.codec.clone())
    }

    pub async fn seed(&self) -> Result<SeededFixture, DevFixtureServiceError> {
        let fixture = FixtureIds::generate();
        let session_id = Uuid::new_v4();
        let access_token = self
            .codec
            .issue(fixture.user_id, session_id)
            .map_err(|_| DevFixtureServiceError::Token)?;
        self.store
            .seed(fixture)
            .await
            .map_err(|_| DevFixtureServiceError::Store)?;

        Ok(SeededFixture {
            user_id: fixture.user_id,
            group_id: fixture.group_id,
            membership_id: fixture.membership_id,
            chatroom_id: fixture.chatroom_id,
            access_token,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SeededFixture {
    pub user_id: Uuid,
    pub group_id: Uuid,
    pub membership_id: Uuid,
    pub chatroom_id: Uuid,
    pub access_token: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DevFixtureServiceError {
    Token,
    Store,
}

impl fmt::Display for DevFixtureServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("dev fixture seed failed")
    }
}

impl Error for DevFixtureServiceError {}

#[cfg(test)]
mod tests {
    use super::{DevFixtureGuard, DevFixtureInitError};

    #[test]
    fn guard_requires_exact_enablement_in_a_non_production_environment() {
        assert!(DevFixtureGuard::from_values(Some("true"), Some("test")).is_ok());
        assert!(DevFixtureGuard::from_values(Some("true"), Some("development")).is_ok());
        for (enabled, environment) in [
            (None, Some("test")),
            (Some("false"), Some("test")),
            (Some("1"), Some("test")),
            (Some("true"), None),
            (Some("true"), Some("production")),
        ] {
            assert_eq!(
                DevFixtureGuard::from_values(enabled, environment),
                Err(DevFixtureInitError::RuntimeGuard)
            );
        }
    }
}
