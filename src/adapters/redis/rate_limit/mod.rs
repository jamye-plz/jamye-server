//! Atomic fixed-window rate limiting shared by all API nodes.

use std::time::Duration;

use redis::Client;
use sha2::{Digest, Sha256};

use crate::ports::rate_limit::{
    RateLimitError, RateLimitFuture, RateLimitOutcome, RateLimitRequest, RateLimiter,
};

const RATE_LIMIT_KEY_PREFIX: &str = "jamye:rate-limit:v1:";
const FIXED_WINDOW_SCRIPT: &str = r#"
local current = redis.call('INCR', KEYS[1])
if current == 1 then
  redis.call('PEXPIRE', KEYS[1], ARGV[1])
end
local ttl = redis.call('PTTL', KEYS[1])
return {current, ttl}
"#;

#[derive(Clone)]
pub struct RedisRateLimiter {
    client: Client,
}

impl RedisRateLimiter {
    pub fn new(redis_url: &str) -> Result<Self, RateLimitError> {
        Client::open(redis_url)
            .map(|client| Self { client })
            .map_err(|_| RateLimitError)
    }

    async fn check_limit(
        &self,
        request: &RateLimitRequest,
    ) -> Result<RateLimitOutcome, RateLimitError> {
        let window_milliseconds = u64::try_from(request.window.as_millis())
            .ok()
            .filter(|window| *window > 0)
            .ok_or(RateLimitError)?;
        if request.limit == 0 || !valid_endpoint(request.endpoint) || request.subject.is_empty() {
            return Err(RateLimitError);
        }
        let mut connection = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|_| redis_failure("rate_limit_connect"))?;
        let (count, ttl_milliseconds) = redis::cmd("EVAL")
            .arg(FIXED_WINDOW_SCRIPT)
            .arg(1)
            .arg(rate_limit_key(request))
            .arg(window_milliseconds)
            .query_async::<(u64, i64)>(&mut connection)
            .await
            .map_err(|_| redis_failure("rate_limit_check"))?;
        if count <= u64::from(request.limit) {
            Ok(RateLimitOutcome::Allowed)
        } else {
            let retry_milliseconds = u64::try_from(ttl_milliseconds)
                .ok()
                .filter(|ttl| *ttl > 0)
                .unwrap_or(1);
            Ok(RateLimitOutcome::Denied {
                retry_after: Duration::from_millis(retry_milliseconds),
            })
        }
    }
}

impl RateLimiter for RedisRateLimiter {
    fn check<'a>(&'a self, request: &'a RateLimitRequest) -> RateLimitFuture<'a> {
        Box::pin(self.check_limit(request))
    }
}

fn valid_endpoint(endpoint: &str) -> bool {
    !endpoint.is_empty()
        && endpoint
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_' || byte.is_ascii_digit())
}

fn rate_limit_key(request: &RateLimitRequest) -> String {
    let subject = Sha256::digest(request.subject.as_bytes());
    format!(
        "{RATE_LIMIT_KEY_PREFIX}{}:{}",
        request.endpoint,
        encode_hex(&subject)
    )
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

fn redis_failure(operation: &'static str) -> RateLimitError {
    tracing::warn!(
        dependency = "redis",
        failure_kind = "rate_limit",
        operation,
        "Redis rate-limit operation failed"
    );
    RateLimitError
}
