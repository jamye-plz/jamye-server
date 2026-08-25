//! Verified access-token identity independent of any concrete issuer.

use std::{error::Error, fmt};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AccessIdentity {
    pub user_id: Uuid,
    pub session_id: Uuid,
    pub issuer: String,
}

impl AccessIdentity {
    pub fn new(user_id: Uuid, session_id: Uuid, issuer: impl Into<String>) -> Self {
        Self {
            user_id,
            session_id,
            issuer: issuer.into(),
        }
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
