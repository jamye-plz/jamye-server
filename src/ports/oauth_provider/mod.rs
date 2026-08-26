//! OAuth provider protocol boundary.

use std::{fmt, future::Future, pin::Pin};

use serde::{Deserialize, Serialize};

pub type OAuthProviderFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, OAuthProviderError>> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Kakao,
    Google,
}

impl ProviderKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "kakao" => Some(Self::Kakao),
            "google" => Some(Self::Google),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kakao => "kakao",
            Self::Google => "google",
        }
    }
}

pub trait OAuthProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;

    fn authorization_url(
        &self,
        request: &AuthorizationRequest,
    ) -> Result<String, OAuthProviderError>;

    fn exchange<'a>(
        &'a self,
        request: &'a ProviderExchangeRequest,
    ) -> OAuthProviderFuture<'a, ProviderIdentity>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationRequest {
    pub redirect_uri: String,
    pub state: String,
    pub code_challenge: String,
    pub nonce: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderExchangeRequest {
    pub authorization_code: String,
    pub redirect_uri: String,
    pub code_verifier: String,
    pub nonce: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderIdentity {
    pub provider_id: String,
    pub nickname: String,
    pub avatar_url: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OAuthProviderError {
    Unavailable,
    InvalidIdentity,
    InvalidConfiguration,
}

impl fmt::Display for OAuthProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OAuth provider operation failed")
    }
}

impl std::error::Error for OAuthProviderError {}
