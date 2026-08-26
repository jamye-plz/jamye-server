//! One-time OAuth attempt coordination without exposing raw Redis access.

use std::{fmt, future::Future, pin::Pin, time::Duration};

use crate::ports::{auth::CredentialDigest, oauth_provider::ProviderKind};

pub type OAuthAttemptFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, OAuthAttemptError>> + Send + 'a>>;

pub trait OAuthAttemptStore: Send + Sync {
    fn create<'a>(
        &'a self,
        state_digest: &'a CredentialDigest,
        attempt: &'a OAuthAttempt,
        ttl: Duration,
    ) -> OAuthAttemptFuture<'a, CreateAttemptOutcome>;

    fn consume<'a>(
        &'a self,
        state_digest: &'a CredentialDigest,
    ) -> OAuthAttemptFuture<'a, ConsumeAttemptOutcome>;
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct OAuthAttempt {
    pub provider: ProviderKind,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub nonce: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateAttemptOutcome {
    Created,
    Collision,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConsumeAttemptOutcome {
    Found(OAuthAttempt),
    Missing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OAuthAttemptError {
    Unavailable,
    InvalidData,
}

impl fmt::Display for OAuthAttemptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OAuth attempt operation failed")
    }
}

impl std::error::Error for OAuthAttemptError {}
