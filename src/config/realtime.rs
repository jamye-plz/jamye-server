//! Feature-local Redis publish timing configuration for the realtime worker.

use std::time::Duration;

use super::ConfigError;

const LEASE_MS_KEY: &str = "JAMYE_REDIS_PUBLISH_LEASE_MS";
const TIMEOUT_MS_KEY: &str = "JAMYE_REDIS_PUBLISH_TIMEOUT_MS";
const SAFETY_MARGIN_MS_KEY: &str = "JAMYE_REDIS_PUBLISH_SAFETY_MARGIN_MS";

const DEFAULT_REDIS_PUBLISH_LEASE_MS: u64 = 15_000;
const DEFAULT_REDIS_PUBLISH_TIMEOUT_MS: u64 = 2_000;
const DEFAULT_REDIS_PUBLISH_SAFETY_MARGIN_MS: u64 = 1_000;

const MIN_REDIS_PUBLISH_LEASE_MS: u64 = 1_000;
const MAX_REDIS_PUBLISH_LEASE_MS: u64 = 300_000;
const MIN_REDIS_PUBLISH_TIMEOUT_MS: u64 = 100;
const MAX_REDIS_PUBLISH_TIMEOUT_MS: u64 = 30_000;
const MIN_REDIS_PUBLISH_SAFETY_MARGIN_MS: u64 = 1;
const MAX_REDIS_PUBLISH_SAFETY_MARGIN_MS: u64 = 30_000;

/// Unvalidated non-secret Redis publish timing input.
#[derive(Clone, Default)]
pub(crate) struct RedisPublishConfigInput {
    pub(crate) lease_ms: Option<String>,
    pub(crate) timeout_ms: Option<String>,
    pub(crate) safety_margin_ms: Option<String>,
}

impl RedisPublishConfigInput {
    pub(crate) fn from_env() -> Self {
        Self {
            lease_ms: super::read_env(LEASE_MS_KEY),
            timeout_ms: super::read_env(TIMEOUT_MS_KEY),
            safety_margin_ms: super::read_env(SAFETY_MARGIN_MS_KEY),
        }
    }
}

/// Validated Redis publish timings used by the realtime outbox worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RedisPublishTiming {
    pub(crate) lease_duration: Duration,
    pub(crate) publish_timeout: Duration,
    pub(crate) lease_safety_margin: Duration,
}

impl RedisPublishTiming {
    pub(crate) fn from_env() -> Result<Self, ConfigError> {
        Self::resolve(RedisPublishConfigInput::from_env())
    }

    #[cfg(test)]
    pub(crate) fn defaults() -> Self {
        Self {
            lease_duration: Duration::from_millis(DEFAULT_REDIS_PUBLISH_LEASE_MS),
            publish_timeout: Duration::from_millis(DEFAULT_REDIS_PUBLISH_TIMEOUT_MS),
            lease_safety_margin: Duration::from_millis(DEFAULT_REDIS_PUBLISH_SAFETY_MARGIN_MS),
        }
    }

    pub(crate) fn resolve(input: RedisPublishConfigInput) -> Result<Self, ConfigError> {
        let lease_duration = milliseconds(
            LEASE_MS_KEY,
            input.lease_ms.as_deref(),
            DEFAULT_REDIS_PUBLISH_LEASE_MS,
            MIN_REDIS_PUBLISH_LEASE_MS,
            MAX_REDIS_PUBLISH_LEASE_MS,
        )?;
        let publish_timeout = milliseconds(
            TIMEOUT_MS_KEY,
            input.timeout_ms.as_deref(),
            DEFAULT_REDIS_PUBLISH_TIMEOUT_MS,
            MIN_REDIS_PUBLISH_TIMEOUT_MS,
            MAX_REDIS_PUBLISH_TIMEOUT_MS,
        )?;
        let lease_safety_margin = milliseconds(
            SAFETY_MARGIN_MS_KEY,
            input.safety_margin_ms.as_deref(),
            DEFAULT_REDIS_PUBLISH_SAFETY_MARGIN_MS,
            MIN_REDIS_PUBLISH_SAFETY_MARGIN_MS,
            MAX_REDIS_PUBLISH_SAFETY_MARGIN_MS,
        )?;

        validate_strict_budget(lease_duration, publish_timeout, lease_safety_margin)?;

        Ok(Self {
            lease_duration,
            publish_timeout,
            lease_safety_margin,
        })
    }
}

fn milliseconds(
    key: &'static str,
    supplied: Option<&str>,
    default: u64,
    minimum: u64,
    maximum: u64,
) -> Result<Duration, ConfigError> {
    let amount = supplied
        .map(str::parse::<u64>)
        .transpose()
        .map_err(|_| ConfigError::new(key, "must be an integer"))?
        .unwrap_or(default);
    if !(minimum..=maximum).contains(&amount) {
        return Err(ConfigError::new(key, "is outside the permitted range"));
    }
    Ok(Duration::from_millis(amount))
}

/// Validates the strict publish lease budget after all input ranges are known.
pub(crate) fn validate_strict_budget(
    lease_duration: Duration,
    publish_timeout: Duration,
    lease_safety_margin: Duration,
) -> Result<(), ConfigError> {
    let publish_budget = publish_timeout
        .checked_add(lease_safety_margin)
        .ok_or_else(|| ConfigError::new(LEASE_MS_KEY, "exceeds the supported duration"))?;
    if lease_duration <= publish_budget {
        return Err(ConfigError::new(
            LEASE_MS_KEY,
            "must exceed publish timeout plus safety margin",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod redis_publish_config_binding {
    use std::time::Duration;

    use super::{
        LEASE_MS_KEY, RedisPublishConfigInput, RedisPublishTiming, SAFETY_MARGIN_MS_KEY,
        TIMEOUT_MS_KEY,
    };

    fn resolved(input: RedisPublishConfigInput) -> RedisPublishTiming {
        match RedisPublishTiming::resolve(input) {
            Ok(timing) => timing,
            Err(error) => panic!("expected valid Redis publish timing for {}", error.key()),
        }
    }

    fn rejected(input: RedisPublishConfigInput) -> crate::config::ConfigError {
        match RedisPublishTiming::resolve(input) {
            Ok(_) => panic!("expected invalid Redis publish timing"),
            Err(error) => error,
        }
    }

    #[test]
    fn defaults_are_exact_15000_2000_1000_ms() {
        let timing = resolved(RedisPublishConfigInput::default());

        assert_eq!(timing.lease_duration, Duration::from_millis(15_000));
        assert_eq!(timing.publish_timeout, Duration::from_millis(2_000));
        assert_eq!(timing.lease_safety_margin, Duration::from_millis(1_000));
    }

    #[test]
    fn key_only_errors_never_echo_raw_values() {
        for (input, expected_key, raw_value) in [
            (
                RedisPublishConfigInput {
                    lease_ms: Some("lease-secret-should-not-echo".to_owned()),
                    ..RedisPublishConfigInput::default()
                },
                LEASE_MS_KEY,
                "lease-secret-should-not-echo",
            ),
            (
                RedisPublishConfigInput {
                    timeout_ms: Some("timeout-secret-should-not-echo".to_owned()),
                    ..RedisPublishConfigInput::default()
                },
                TIMEOUT_MS_KEY,
                "timeout-secret-should-not-echo",
            ),
            (
                RedisPublishConfigInput {
                    safety_margin_ms: Some("margin-secret-should-not-echo".to_owned()),
                    ..RedisPublishConfigInput::default()
                },
                SAFETY_MARGIN_MS_KEY,
                "margin-secret-should-not-echo",
            ),
        ] {
            let error = rejected(input);

            assert_eq!(error.key(), expected_key);
            assert!(!error.to_string().contains(raw_value));
        }
    }

    #[test]
    fn partial_override_uses_remaining_defaults() {
        for (input, expected) in [
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
            let timing = resolved(input);

            assert_eq!(timing.lease_duration, Duration::from_millis(expected.0));
            assert_eq!(timing.publish_timeout, Duration::from_millis(expected.1));
            assert_eq!(
                timing.lease_safety_margin,
                Duration::from_millis(expected.2)
            );
        }
    }
}
