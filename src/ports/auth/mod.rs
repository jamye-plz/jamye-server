//! Authentication persistence, token, clock, and credential boundaries.

use std::{fmt, future::Future, pin::Pin, time::Duration};

use time::OffsetDateTime;
use uuid::Uuid;

use crate::ports::transactions::TransactionHandle;

pub type AuthRepositoryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, AuthRepositoryError>> + Send + 'a>>;

pub trait AuthRepository: Send + Sync {
    fn create_session<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        identity: &'a NewProviderIdentity,
        session: &'a NewRefreshSession,
    ) -> AuthRepositoryFuture<'a, IssuedSession>;

    fn rotate_session<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        token_hash: &'a CredentialDigest,
        child: &'a NewRotatedSession,
        now: OffsetDateTime,
    ) -> AuthRepositoryFuture<'a, RotationOutcome>;

    fn revoke_session<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        session_id: Uuid,
        now: OffsetDateTime,
    ) -> AuthRepositoryFuture<'a, ()>;

    fn profile(&self, user_id: Uuid) -> AuthRepositoryFuture<'_, Option<UserProfile>>;

    fn update_profile<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        user_id: Uuid,
        patch: &'a ProfilePatch,
    ) -> AuthRepositoryFuture<'a, Option<UserProfile>>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthRepositoryError {
    Unavailable,
    InvalidData,
}

impl fmt::Display for AuthRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("authentication repository operation failed")
    }
}

impl std::error::Error for AuthRepositoryError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewProviderIdentity {
    pub provider: String,
    pub provider_id: String,
    pub nickname: String,
    pub avatar_url: Option<String>,
}

#[derive(Clone, Eq, PartialEq)]
pub struct CredentialDigest([u8; 32]);

impl CredentialDigest {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for CredentialDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CredentialDigest([REDACTED])")
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct RawCredential(String);

impl RawCredential {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Debug for RawCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RawCredential([REDACTED])")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedCredential {
    pub raw: RawCredential,
    pub digest: CredentialDigest,
}

pub trait CredentialSource: Send + Sync {
    fn generate(&self) -> Result<GeneratedCredential, CredentialError>;
    fn digest(&self, raw: &str) -> Result<CredentialDigest, CredentialError>;
    fn pkce_s256(&self, verifier: &str) -> Result<String, CredentialError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CredentialError;

impl fmt::Display for CredentialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("credential operation failed")
    }
}

impl std::error::Error for CredentialError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewRefreshSession {
    pub id: Uuid,
    pub family_id: Uuid,
    pub parent_session_id: Option<Uuid>,
    pub token_hash: CredentialDigest,
    pub expires_at: OffsetDateTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewRotatedSession {
    pub id: Uuid,
    pub token_hash: CredentialDigest,
    pub expires_at: OffsetDateTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IssuedSession {
    pub user_id: Uuid,
    pub session_id: Uuid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RotationOutcome {
    Rotated(IssuedSession),
    Invalid,
    Reused,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserProfile {
    pub id: Uuid,
    pub provider: String,
    pub nickname: String,
    pub avatar_url: Option<String>,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfilePatch {
    pub nickname: Option<String>,
    pub avatar_url: AvatarPatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AvatarPatch {
    Unchanged,
    Set(String),
    Clear,
}

pub trait AccessTokenIssuer: Send + Sync {
    fn issue(
        &self,
        user_id: Uuid,
        session_id: Uuid,
        issued_at: OffsetDateTime,
        expires_at: OffsetDateTime,
    ) -> Result<String, AccessTokenIssueError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccessTokenIssueError;

impl fmt::Display for AccessTokenIssueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("access token issuance failed")
    }
}

impl std::error::Error for AccessTokenIssueError {}

pub trait AuthClock: Send + Sync {
    fn now(&self) -> OffsetDateTime;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthLifetimePolicy {
    pub access: Duration,
    pub refresh: Duration,
}
