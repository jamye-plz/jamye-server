//! Sign in with Apple identity verification and account revocation boundary.

use std::{fmt, future::Future, pin::Pin};

pub type AppleIdentityProviderFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, AppleIdentityProviderError>> + Send + 'a>>;

pub type AppleRevocationProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), AppleRevocationProviderError>> + Send + 'a>>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppleIdentityVerificationRequest {
    pub identity_token: String,
    pub raw_nonce: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppleIdentity {
    pub provider_id: String,
    pub client_id: String,
}

pub trait AppleIdentityProvider: Send + Sync {
    fn verify_identity<'a>(
        &'a self,
        request: &'a AppleIdentityVerificationRequest,
    ) -> AppleIdentityProviderFuture<'a, AppleIdentity>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppleAuthorizationCodeRevocationRequest {
    pub client_id: String,
    pub authorization_code: String,
}

pub trait AppleRevocationProvider: Send + Sync {
    fn revoke_authorization_code<'a>(
        &'a self,
        request: &'a AppleAuthorizationCodeRevocationRequest,
    ) -> AppleRevocationProviderFuture<'a>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppleIdentityProviderError {
    Unavailable,
    InvalidIdentity,
    InvalidConfiguration,
}

impl fmt::Display for AppleIdentityProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Apple identity verification failed")
    }
}

impl std::error::Error for AppleIdentityProviderError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppleRevocationProviderError {
    Unavailable,
    InvalidAuthorizationCode,
    InvalidConfiguration,
}

impl fmt::Display for AppleRevocationProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Apple authorization-code revocation failed")
    }
}

impl std::error::Error for AppleRevocationProviderError {}
