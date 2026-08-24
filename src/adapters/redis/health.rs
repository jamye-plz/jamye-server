//! Redis readiness probe.

use std::{error::Error, fmt, time::Duration};

use crate::platform::readiness::{DependencyProbe, ProbeFuture, ProbeOutcome};

pub struct RedisHealthProbe {
    client: redis::Client,
    timeout: Duration,
}

impl RedisHealthProbe {
    pub fn new(redis_url: &str, timeout: Duration) -> Result<Self, RedisInitError> {
        let client = redis::Client::open(redis_url).map_err(|_| RedisInitError)?;
        Ok(Self { client, timeout })
    }
}

impl DependencyProbe for RedisHealthProbe {
    fn check(&self) -> ProbeFuture<'_> {
        Box::pin(async move {
            let check = async {
                let mut connection = self.client.get_multiplexed_async_connection().await?;
                redis::cmd("PING")
                    .query_async::<String>(&mut connection)
                    .await
            };
            match tokio::time::timeout(self.timeout, check).await {
                Ok(Ok(response)) if response == "PONG" => ProbeOutcome::Reachable,
                Ok(Ok(_)) => {
                    log_failure("unexpected_response");
                    ProbeOutcome::Unreachable
                }
                Ok(Err(_)) => {
                    log_failure("command");
                    ProbeOutcome::Unreachable
                }
                Err(_) => {
                    log_failure("timeout");
                    ProbeOutcome::Unreachable
                }
            }
        })
    }
}

fn log_failure(failure_kind: &'static str) {
    tracing::warn!(
        dependency = "redis",
        failure_kind,
        "dependency readiness check failed"
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RedisInitError;

impl fmt::Display for RedisInitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("REDIS_URL is not a valid Redis connection URL")
    }
}

impl Error for RedisInitError {}
