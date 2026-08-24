//! Readiness policy for required and degradable backing services.

use std::{future::Future, pin::Pin, sync::Arc};

pub type ProbeFuture<'a> = Pin<Box<dyn Future<Output = ProbeOutcome> + Send + 'a>>;

/// Minimal replaceable boundary for a backing-service readiness check.
pub trait DependencyProbe: Send + Sync {
    fn check(&self) -> ProbeFuture<'_>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeOutcome {
    Reachable,
    Unreachable,
}

/// Combined dependency report before transport-specific serialization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadinessReport {
    pub postgres: ProbeOutcome,
    pub redis: ProbeOutcome,
    pub minio: ProbeOutcome,
}

impl ReadinessReport {
    pub fn is_ready(self) -> bool {
        self.postgres == ProbeOutcome::Reachable
    }
}

/// Executes independent checks concurrently and owns required/degraded policy.
#[derive(Clone)]
pub struct ReadinessService {
    postgres: Arc<dyn DependencyProbe>,
    redis: Arc<dyn DependencyProbe>,
    minio: Arc<dyn DependencyProbe>,
}

impl ReadinessService {
    pub fn new(
        postgres: Arc<dyn DependencyProbe>,
        redis: Arc<dyn DependencyProbe>,
        minio: Arc<dyn DependencyProbe>,
    ) -> Self {
        Self {
            postgres,
            redis,
            minio,
        }
    }

    pub async fn check(&self) -> ReadinessReport {
        let (postgres, redis, minio) = tokio::join!(
            self.postgres.check(),
            self.redis.check(),
            self.minio.check()
        );
        ReadinessReport {
            postgres,
            redis,
            minio,
        }
    }
}

/// Represents an optional dependency that has not been configured.
pub struct UnconfiguredProbe;

impl DependencyProbe for UnconfiguredProbe {
    fn check(&self) -> ProbeFuture<'_> {
        Box::pin(async { ProbeOutcome::Unreachable })
    }
}
