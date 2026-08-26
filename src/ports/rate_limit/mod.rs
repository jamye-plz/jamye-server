//! Shared atomic rate-limit boundary.

use std::{fmt, future::Future, pin::Pin, time::Duration};

pub type RateLimitFuture<'a> =
    Pin<Box<dyn Future<Output = Result<RateLimitOutcome, RateLimitError>> + Send + 'a>>;

pub trait RateLimiter: Send + Sync {
    fn check<'a>(&'a self, request: &'a RateLimitRequest) -> RateLimitFuture<'a>;
}

#[derive(Clone, Eq, PartialEq)]
pub struct RateLimitRequest {
    pub endpoint: &'static str,
    pub subject: String,
    pub limit: u32,
    pub window: Duration,
}

impl fmt::Debug for RateLimitRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RateLimitRequest")
            .field("endpoint", &self.endpoint)
            .field("subject", &"[REDACTED]")
            .field("limit", &self.limit)
            .field("window", &self.window)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RateLimitOutcome {
    Allowed,
    Denied { retry_after: Duration },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RateLimitError;

impl fmt::Display for RateLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("rate-limit operation failed")
    }
}

impl std::error::Error for RateLimitError {}
