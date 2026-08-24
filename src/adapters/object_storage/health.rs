//! MinIO liveness-endpoint readiness probe.

use std::{error::Error, fmt, time::Duration};

use reqwest::{Client, Url, redirect::Policy};

use crate::platform::readiness::{DependencyProbe, ProbeFuture, ProbeOutcome};

pub struct MinioHealthProbe {
    client: Client,
    health_url: Url,
    timeout: Duration,
}

impl MinioHealthProbe {
    pub fn new(health_url: &str, timeout: Duration) -> Result<Self, MinioInitError> {
        let health_url = Url::parse(health_url).map_err(|_| MinioInitError)?;
        let client = Client::builder()
            .connect_timeout(timeout)
            .timeout(timeout)
            .redirect(Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| MinioInitError)?;
        Ok(Self {
            client,
            health_url,
            timeout,
        })
    }
}

impl DependencyProbe for MinioHealthProbe {
    fn check(&self) -> ProbeFuture<'_> {
        Box::pin(async move {
            let request = self.client.get(self.health_url.clone()).send();
            match tokio::time::timeout(self.timeout, request).await {
                Ok(Ok(response)) if response.status().is_success() => ProbeOutcome::Reachable,
                Ok(Ok(_)) => {
                    log_failure("status");
                    ProbeOutcome::Unreachable
                }
                Ok(Err(_)) => {
                    log_failure("request");
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
        dependency = "minio",
        failure_kind,
        "dependency readiness check failed"
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MinioInitError;

impl fmt::Display for MinioInitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JAMYE_MINIO_HEALTH_URL could not initialize its HTTP client")
    }
}

impl Error for MinioInitError {}
