use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use std::fmt;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    application::auth::{AccessIdentity, AccessTokenVerifier, AuthenticationError},
    ports::auth::{AccessTokenIssueError, AccessTokenIssuer},
};

const DEV_ISSUER: &str = "jamye-dev";

pub struct ProductionTokenCodec {
    issuer: String,
    audience: String,
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
}

impl ProductionTokenCodec {
    pub fn new(
        secret: &[u8],
        issuer: impl Into<String>,
        audience: impl Into<String>,
    ) -> Result<Self, ProductionTokenConfigError> {
        let issuer = issuer.into();
        let audience = audience.into();
        if secret.len() < 32
            || issuer.trim().is_empty()
            || issuer == DEV_ISSUER
            || audience.trim().is_empty()
        {
            return Err(ProductionTokenConfigError);
        }
        Ok(Self {
            issuer,
            audience,
            encoding_key: EncodingKey::from_secret(secret),
            decoding_key: DecodingKey::from_secret(secret),
        })
    }

    fn validation(&self) -> Validation {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_audience(&[self.audience.as_str()]);
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.set_required_spec_claims(&["sub", "iss", "aud", "iat", "exp"]);
        validation
    }
}

impl AccessTokenIssuer for ProductionTokenCodec {
    fn issue(
        &self,
        user_id: Uuid,
        session_id: Uuid,
        issued_at: OffsetDateTime,
        expires_at: OffsetDateTime,
    ) -> Result<String, AccessTokenIssueError> {
        let iat = u64::try_from(issued_at.unix_timestamp()).map_err(|_| AccessTokenIssueError)?;
        let exp = u64::try_from(expires_at.unix_timestamp()).map_err(|_| AccessTokenIssueError)?;
        if exp <= iat {
            return Err(AccessTokenIssueError);
        }
        encode(
            &Header::new(Algorithm::HS256),
            &AccessClaims {
                sub: user_id,
                sid: session_id,
                iss: self.issuer.clone(),
                aud: self.audience.clone(),
                iat,
                exp,
            },
            &self.encoding_key,
        )
        .map_err(|_| AccessTokenIssueError)
    }
}

impl AccessTokenVerifier for ProductionTokenCodec {
    fn verify(&self, token: &str) -> Result<AccessIdentity, AuthenticationError> {
        let claims = decode::<AccessClaims>(token, &self.decoding_key, &self.validation())
            .map_err(|_| AuthenticationError)?
            .claims;
        let expires_at = OffsetDateTime::from_unix_timestamp(
            i64::try_from(claims.exp).map_err(|_| AuthenticationError)?,
        )
        .map_err(|_| AuthenticationError)?;
        Ok(AccessIdentity::new(claims.sub, claims.sid, claims.iss)
            .with_access_token_expiry(expires_at))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct AccessClaims {
    sub: Uuid,
    sid: Uuid,
    iss: String,
    aud: String,
    iat: u64,
    exp: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProductionTokenConfigError;

impl fmt::Display for ProductionTokenConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("production token configuration is invalid")
    }
}

impl std::error::Error for ProductionTokenConfigError {}
