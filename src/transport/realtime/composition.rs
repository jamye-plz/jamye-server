//! Static worker composition for PostgreSQL outbox delivery through Redis.

use std::{error::Error, fmt, future::Future, sync::Arc, time::Duration};

use uuid::Uuid;

use crate::{
    adapters::{
        postgres::{realtime::PostgresRealtimeRepository, runtime_pool},
        redis::realtime::RedisRealtimeAdapter,
    },
    application::realtime::{OutboxWorker, OutboxWorkerConfig},
    config::{AppConfig, realtime::RedisPublishTiming},
};

pub struct WorkerRuntime {
    worker: OutboxWorker,
}

impl WorkerRuntime {
    pub async fn run_until<F>(self, shutdown: F)
    where
        F: Future<Output = ()> + Send,
    {
        tokio::pin!(shutdown);
        loop {
            match self.worker.run_once().await {
                Ok(report) if report.claimed > 0 => {
                    tracing::info!(
                        claimed = report.claimed,
                        published = report.published,
                        retries = report.retries,
                        dead_lettered = report.dead_lettered,
                        stale_claims = report.stale_claims,
                        "outbox worker batch completed"
                    );
                }
                Ok(_) => {}
                Err(_) => {
                    tracing::warn!(
                        dependency = "postgres",
                        failure_kind = "worker_poll",
                        "outbox worker poll failed"
                    );
                }
            }
            tokio::select! {
                () = &mut shutdown => break,
                () = tokio::time::sleep(self.worker.poll_interval()) => {}
            }
        }
    }
}

pub fn worker(config: &AppConfig) -> Result<WorkerRuntime, WorkerCompositionError> {
    worker_with_factory(
        || RedisPublishTiming::from_env().map_err(|_| WorkerCompositionError::Worker),
        |timing| ProductionWorkerFactory::build(config, timing),
    )
}

/// Builds the realtime worker through the production factory only after the
/// feature-local publish timing resolves successfully.
fn worker_with_factory<R, F>(
    resolve_timing: R,
    factory: F,
) -> Result<WorkerRuntime, WorkerCompositionError>
where
    R: FnOnce() -> Result<RedisPublishTiming, WorkerCompositionError>,
    F: FnOnce(RedisPublishTiming) -> Result<WorkerRuntime, WorkerCompositionError>,
{
    let timing = resolve_timing()?;
    factory(timing)
}

struct ProductionWorkerFactory;

impl ProductionWorkerFactory {
    fn build(
        config: &AppConfig,
        timing: RedisPublishTiming,
    ) -> Result<WorkerRuntime, WorkerCompositionError> {
        let pool = runtime_pool(config.database_url(), config.readiness_timeout())
            .map_err(|_| WorkerCompositionError::Postgres)?;
        let redis_url = config
            .redis_url()
            .ok_or(WorkerCompositionError::RedisNotConfigured)?;
        let redis = Arc::new(
            RedisRealtimeAdapter::new(redis_url).map_err(|_| WorkerCompositionError::Redis)?,
        );
        let repository = Arc::new(PostgresRealtimeRepository::new(pool));
        let worker = OutboxWorker::new(
            repository,
            redis,
            outbox_worker_config(timing, format!("worker-{}", Uuid::new_v4())),
        )
        .map_err(|_| WorkerCompositionError::Worker)?;
        Ok(WorkerRuntime { worker })
    }
}

fn outbox_worker_config(timing: RedisPublishTiming, claim_owner: String) -> OutboxWorkerConfig {
    OutboxWorkerConfig {
        claim_owner,
        batch_size: 50,
        lease_duration: timing.lease_duration,
        publish_timeout: timing.publish_timeout,
        lease_safety_margin: timing.lease_safety_margin,
        retry_delay: Duration::from_secs(1),
        poll_interval: Duration::from_millis(250),
        max_attempts: 8,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerCompositionError {
    Postgres,
    RedisNotConfigured,
    Redis,
    Worker,
}

impl fmt::Display for WorkerCompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("failed to initialize the realtime worker")
    }
}

impl Error for WorkerCompositionError {}

#[cfg(test)]
mod redis_publish_config_binding {
    use std::time::Duration;

    use crate::config::realtime::{
        RedisPublishConfigInput, RedisPublishTiming, validate_strict_budget,
    };

    use super::{WorkerCompositionError, outbox_worker_config, worker_with_factory};

    fn assert_invalid_input_stops_factory(input: RedisPublishConfigInput, expected_key: &str) {
        let error = match RedisPublishTiming::resolve(input.clone()) {
            Ok(_) => panic!("expected invalid Redis publish timing"),
            Err(error) => error,
        };
        assert_eq!(error.key(), expected_key);

        let mut factory_calls = 0;
        let result = worker_with_factory(
            || RedisPublishTiming::resolve(input).map_err(|_| WorkerCompositionError::Worker),
            |_| {
                factory_calls += 1;
                Err(WorkerCompositionError::Worker)
            },
        );

        assert!(matches!(result, Err(WorkerCompositionError::Worker)));
        assert_eq!(factory_calls, 0);
    }

    #[test]
    fn all_three_overrides_materialize_outbox_worker_durations() {
        for (input, expected) in [
            (
                RedisPublishConfigInput {
                    lease_ms: Some("25000".to_owned()),
                    timeout_ms: Some("4000".to_owned()),
                    safety_margin_ms: Some("2000".to_owned()),
                },
                (25_000, 4_000, 2_000),
            ),
            (
                RedisPublishConfigInput {
                    lease_ms: Some("1000".to_owned()),
                    timeout_ms: Some("100".to_owned()),
                    safety_margin_ms: Some("1".to_owned()),
                },
                (1_000, 100, 1),
            ),
            (
                RedisPublishConfigInput {
                    lease_ms: Some("300000".to_owned()),
                    timeout_ms: Some("30000".to_owned()),
                    safety_margin_ms: Some("30000".to_owned()),
                },
                (300_000, 30_000, 30_000),
            ),
            (
                RedisPublishConfigInput {
                    lease_ms: Some("16000".to_owned()),
                    ..RedisPublishConfigInput::default()
                },
                (16_000, 2_000, 1_000),
            ),
            (
                RedisPublishConfigInput {
                    timeout_ms: Some("2100".to_owned()),
                    ..RedisPublishConfigInput::default()
                },
                (15_000, 2_100, 1_000),
            ),
            (
                RedisPublishConfigInput {
                    safety_margin_ms: Some("1100".to_owned()),
                    ..RedisPublishConfigInput::default()
                },
                (15_000, 2_000, 1_100),
            ),
        ] {
            let mut captured = None;
            let result = worker_with_factory(
                || RedisPublishTiming::resolve(input).map_err(|_| WorkerCompositionError::Worker),
                |timing| {
                    captured = Some(outbox_worker_config(
                        timing,
                        "realtime-green-capture".to_owned(),
                    ));
                    Err(WorkerCompositionError::Worker)
                },
            );

            assert!(matches!(result, Err(WorkerCompositionError::Worker)));
            let Some(config) = captured else {
                panic!("valid timing must reach the factory capture");
            };
            assert_eq!(config.lease_duration, Duration::from_millis(expected.0));
            assert_eq!(config.publish_timeout, Duration::from_millis(expected.1));
            assert_eq!(
                config.lease_safety_margin,
                Duration::from_millis(expected.2)
            );
        }
    }

    #[test]
    fn arithmetic_overflow_fails_before_side_effects() {
        let overflowing_timeout = Duration::new(u64::MAX, 999_999_999);
        let overflowing_margin = Duration::from_nanos(1);
        let mut factory_calls = 0;
        let result = worker_with_factory(
            || {
                validate_strict_budget(
                    Duration::from_secs(1),
                    overflowing_timeout,
                    overflowing_margin,
                )
                .map(|_| RedisPublishTiming::defaults())
                .map_err(|_| WorkerCompositionError::Worker)
            },
            |_| {
                factory_calls += 1;
                Err(WorkerCompositionError::Worker)
            },
        );

        assert!(matches!(result, Err(WorkerCompositionError::Worker)));
        assert_eq!(factory_calls, 0);
    }

    #[test]
    fn equality_and_over_budget_fail_before_side_effects() {
        for (input, expected_result) in [
            (
                RedisPublishConfigInput {
                    lease_ms: Some("2999".to_owned()),
                    timeout_ms: Some("2000".to_owned()),
                    safety_margin_ms: Some("1000".to_owned()),
                },
                Err("JAMYE_REDIS_PUBLISH_LEASE_MS"),
            ),
            (
                RedisPublishConfigInput {
                    lease_ms: Some("3000".to_owned()),
                    timeout_ms: Some("2000".to_owned()),
                    safety_margin_ms: Some("1000".to_owned()),
                },
                Err("JAMYE_REDIS_PUBLISH_LEASE_MS"),
            ),
            (
                RedisPublishConfigInput {
                    lease_ms: Some("3001".to_owned()),
                    timeout_ms: Some("2000".to_owned()),
                    safety_margin_ms: Some("1000".to_owned()),
                },
                Ok(()),
            ),
            (
                RedisPublishConfigInput {
                    lease_ms: Some("3000".to_owned()),
                    timeout_ms: Some("2000".to_owned()),
                    safety_margin_ms: Some("1001".to_owned()),
                },
                Err("JAMYE_REDIS_PUBLISH_LEASE_MS"),
            ),
        ] {
            match expected_result {
                Err(expected_key) => assert_invalid_input_stops_factory(input, expected_key),
                Ok(()) => {
                    let timing = match RedisPublishTiming::resolve(input) {
                        Ok(timing) => timing,
                        Err(error) => panic!("expected strict budget success for {}", error.key()),
                    };
                    assert_eq!(timing.lease_duration, Duration::from_millis(3_001));
                }
            }
        }
    }

    #[test]
    fn existing_realtime_worker_default_regression() {
        let config = outbox_worker_config(
            RedisPublishTiming::defaults(),
            "realtime-default-regression".to_owned(),
        );

        assert_eq!(config.lease_duration, Duration::from_millis(15_000));
        assert_eq!(config.publish_timeout, Duration::from_millis(2_000));
        assert_eq!(config.lease_safety_margin, Duration::from_millis(1_000));
    }

    #[test]
    fn invalid_parse_and_bounds_fail_before_side_effects() {
        for (input, expected_key) in [
            (
                RedisPublishConfigInput {
                    lease_ms: Some("0".to_owned()),
                    ..RedisPublishConfigInput::default()
                },
                "JAMYE_REDIS_PUBLISH_LEASE_MS",
            ),
            (
                RedisPublishConfigInput {
                    lease_ms: Some("-1".to_owned()),
                    ..RedisPublishConfigInput::default()
                },
                "JAMYE_REDIS_PUBLISH_LEASE_MS",
            ),
            (
                RedisPublishConfigInput {
                    lease_ms: Some("999".to_owned()),
                    ..RedisPublishConfigInput::default()
                },
                "JAMYE_REDIS_PUBLISH_LEASE_MS",
            ),
            (
                RedisPublishConfigInput {
                    lease_ms: Some("300001".to_owned()),
                    ..RedisPublishConfigInput::default()
                },
                "JAMYE_REDIS_PUBLISH_LEASE_MS",
            ),
            (
                RedisPublishConfigInput {
                    timeout_ms: Some("not-an-integer".to_owned()),
                    ..RedisPublishConfigInput::default()
                },
                "JAMYE_REDIS_PUBLISH_TIMEOUT_MS",
            ),
            (
                RedisPublishConfigInput {
                    timeout_ms: Some("18446744073709551616".to_owned()),
                    ..RedisPublishConfigInput::default()
                },
                "JAMYE_REDIS_PUBLISH_TIMEOUT_MS",
            ),
            (
                RedisPublishConfigInput {
                    timeout_ms: Some("99".to_owned()),
                    ..RedisPublishConfigInput::default()
                },
                "JAMYE_REDIS_PUBLISH_TIMEOUT_MS",
            ),
            (
                RedisPublishConfigInput {
                    timeout_ms: Some("30001".to_owned()),
                    ..RedisPublishConfigInput::default()
                },
                "JAMYE_REDIS_PUBLISH_TIMEOUT_MS",
            ),
            (
                RedisPublishConfigInput {
                    safety_margin_ms: Some("0".to_owned()),
                    ..RedisPublishConfigInput::default()
                },
                "JAMYE_REDIS_PUBLISH_SAFETY_MARGIN_MS",
            ),
            (
                RedisPublishConfigInput {
                    safety_margin_ms: Some("30001".to_owned()),
                    ..RedisPublishConfigInput::default()
                },
                "JAMYE_REDIS_PUBLISH_SAFETY_MARGIN_MS",
            ),
            (
                RedisPublishConfigInput {
                    lease_ms: Some("1000".to_owned()),
                    timeout_ms: Some("99".to_owned()),
                    safety_margin_ms: Some("30000".to_owned()),
                },
                "JAMYE_REDIS_PUBLISH_TIMEOUT_MS",
            ),
        ] {
            assert_invalid_input_stops_factory(input, expected_key);
        }
    }
}
