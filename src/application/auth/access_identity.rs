//! Verified access-token identity independent of any concrete issuer.

use std::{error::Error, fmt};

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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthenticationError;

impl fmt::Display for AuthenticationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("access token is not valid")
    }
}

impl Error for AuthenticationError {}
