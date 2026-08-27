//! Digest-only one-time OAuth attempt storage.

use std::time::Duration;

use redis::Client;

use crate::ports::{
    auth::CredentialDigest,
    oauth_attempt::{
        ConsumeAttemptOutcome, CreateAttemptOutcome, OAuthAttempt, OAuthAttemptError,
        OAuthAttemptFuture, OAuthAttemptStore,
    },
};

const ATTEMPT_KEY_PREFIX: &str = "jamye:oauth-attempt:v1:";

#[derive(Clone)]
pub struct RedisOAuthAttemptStore {
    client: Client,
}

impl RedisOAuthAttemptStore {
    pub fn new(redis_url: &str) -> Result<Self, OAuthAttemptError> {
        Client::open(redis_url)
            .map(|client| Self { client })
            .map_err(|_| OAuthAttemptError::Unavailable)
    }

    async fn create_attempt(
        &self,
        state_digest: &CredentialDigest,
        attempt: &OAuthAttempt,
        ttl: Duration,
    ) -> Result<CreateAttemptOutcome, OAuthAttemptError> {
        let ttl_seconds = ttl.as_secs();
        if ttl_seconds == 0 {
            return Err(OAuthAttemptError::InvalidData);
        }
        let payload = serde_json::to_string(attempt).map_err(|_| OAuthAttemptError::InvalidData)?;
        let mut connection = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|_| redis_failure("oauth_attempt_connect"))?;
        let stored = redis::cmd("SET")
            .arg(attempt_key(state_digest))
            .arg(payload)
            .arg("NX")
            .arg("EX")
            .arg(ttl_seconds)
            .query_async::<Option<String>>(&mut connection)
            .await
            .map_err(|_| redis_failure("oauth_attempt_create"))?;
        Ok(if stored.is_some() {
            CreateAttemptOutcome::Created
        } else {
            CreateAttemptOutcome::Collision
        })
    }

    async fn consume_attempt(
        &self,
        state_digest: &CredentialDigest,
    ) -> Result<ConsumeAttemptOutcome, OAuthAttemptError> {
        let mut connection = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|_| redis_failure("oauth_attempt_connect"))?;
        let payload = redis::cmd("GETDEL")
            .arg(attempt_key(state_digest))
            .query_async::<Option<String>>(&mut connection)
            .await
            .map_err(|_| redis_failure("oauth_attempt_consume"))?;
        payload.map_or(Ok(ConsumeAttemptOutcome::Missing), |payload| {
            serde_json::from_str(&payload)
                .map(ConsumeAttemptOutcome::Found)
                .map_err(|_| OAuthAttemptError::InvalidData)
        })
    }
}

impl OAuthAttemptStore for RedisOAuthAttemptStore {
    fn create<'a>(
        &'a self,
        state_digest: &'a CredentialDigest,
        attempt: &'a OAuthAttempt,
        ttl: Duration,
    ) -> OAuthAttemptFuture<'a, CreateAttemptOutcome> {
        Box::pin(self.create_attempt(state_digest, attempt, ttl))
    }

    fn consume<'a>(
        &'a self,
        state_digest: &'a CredentialDigest,
    ) -> OAuthAttemptFuture<'a, ConsumeAttemptOutcome> {
        Box::pin(self.consume_attempt(state_digest))
    }
}

fn attempt_key(digest: &CredentialDigest) -> String {
    format!("{ATTEMPT_KEY_PREFIX}{}", encode_hex(digest.as_bytes()))
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn redis_failure(operation: &'static str) -> OAuthAttemptError {
    tracing::warn!(
        dependency = "redis",
        failure_kind = "oauth_attempt",
        operation,
        "Redis OAuth attempt operation failed"
    );
    OAuthAttemptError::Unavailable
}
