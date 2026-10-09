//! Verified access-token identity independent of any concrete issuer.

use std::{error::Error, fmt, future::Future, pin::Pin, sync::Arc};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AccessIdentity {
    pub user_id: Uuid,
    pub session_id: Uuid,
    pub issuer: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub access_token_expires_at: Option<OffsetDateTime>,
}

impl AccessIdentity {
    pub fn new(user_id: Uuid, session_id: Uuid, issuer: impl Into<String>) -> Self {
        Self {
            user_id,
            session_id,
            issuer: issuer.into(),
            access_token_expires_at: None,
        }
    }

    pub fn with_access_token_expiry(mut self, expires_at: OffsetDateTime) -> Self {
        self.access_token_expires_at = Some(expires_at);
        self
    }
}

pub trait AccessTokenVerifier: Send + Sync {
    fn verify(&self, token: &str) -> Result<AccessIdentity, AuthenticationError>;

    /// Account-state gate the bearer extractor awaits after a token verifies (suspension).
    /// Verifiers without a gate accept every verified token.
    fn account_gate(&self) -> Option<Arc<dyn AccessAccountGate>> {
        None
    }
}

pub type AccessGateFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), AccessGateError>> + Send + 'a>>;

/// Per-request account check that needs persistence, so it cannot live in the stateless
/// token verification itself.
pub trait AccessAccountGate: Send + Sync {
    fn check<'a>(&'a self, user_id: Uuid) -> AccessGateFuture<'a>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessGateError {
    Suspended,
    Unavailable,
}

/// Wraps a verifier and attaches the account gate. Token verification is unchanged.
#[derive(Clone)]
pub struct GatedAccessTokenVerifier {
    inner: Arc<dyn AccessTokenVerifier>,
    gate: Arc<dyn AccessAccountGate>,
}

impl GatedAccessTokenVerifier {
    pub fn new(inner: Arc<dyn AccessTokenVerifier>, gate: Arc<dyn AccessAccountGate>) -> Self {
        Self { inner, gate }
    }
}

impl AccessTokenVerifier for GatedAccessTokenVerifier {
    fn verify(&self, token: &str) -> Result<AccessIdentity, AuthenticationError> {
        self.inner.verify(token)
    }

    fn account_gate(&self) -> Option<Arc<dyn AccessAccountGate>> {
        Some(self.gate.clone())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthenticationError;

impl fmt::Display for AuthenticationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("access token is not valid")
    }
}

impl Error for AuthenticationError {}
