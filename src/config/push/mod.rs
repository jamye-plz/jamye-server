//! Feature-local Expo delivery configuration.

use std::{env, fmt, time::Duration};

use crate::adapters::push::expo::{
    EXPO_PUSH_SEND_URL, valid_access_token_configuration, valid_endpoint_configuration,
};

use super::{AppEnvironment, ConfigError};

const ENDPOINT_KEY: &str = "JAMYE_EXPO_PUSH_SEND_URL";
const ACCESS_TOKEN_KEY: &str = "JAMYE_EXPO_ACCESS_TOKEN";
const BATCH_SIZE_KEY: &str = "JAMYE_PUSH_BATCH_SIZE";
const LEASE_MS_KEY: &str = "JAMYE_PUSH_LEASE_MS";
const PROVIDER_TIMEOUT_MS_KEY: &str = "JAMYE_PUSH_PROVIDER_TIMEOUT_MS";
const LEASE_SAFETY_MARGIN_MS_KEY: &str = "JAMYE_PUSH_LEASE_SAFETY_MARGIN_MS";
const RETRY_DELAY_MS_KEY: &str = "JAMYE_PUSH_RETRY_DELAY_MS";
const POLL_INTERVAL_MS_KEY: &str = "JAMYE_PUSH_POLL_INTERVAL_MS";
const MAX_ATTEMPTS_KEY: &str = "JAMYE_PUSH_MAX_ATTEMPTS";

const DEFAULT_BATCH_SIZE: &str = "50";
const DEFAULT_LEASE_MS: &str = "15000";
const DEFAULT_PROVIDER_TIMEOUT_MS: &str = "2000";
const DEFAULT_LEASE_SAFETY_MARGIN_MS: &str = "1000";
const DEFAULT_RETRY_DELAY_MS: &str = "1000";
const DEFAULT_POLL_INTERVAL_MS: &str = "250";
const DEFAULT_MAX_ATTEMPTS: &str = "8";

#[derive(Clone, Default)]
pub struct PushConfigInput {
    pub endpoint: Option<String>,
    pub access_token: Option<String>,
    pub batch_size: Option<String>,
    pub lease_ms: Option<String>,
    pub provider_timeout_ms: Option<String>,
    pub lease_safety_margin_ms: Option<String>,
    pub retry_delay_ms: Option<String>,
    pub poll_interval_ms: Option<String>,
    pub max_attempts: Option<String>,
}

impl PushConfigInput {
    pub fn from_env() -> Self {
        Self {
            endpoint: read(ENDPOINT_KEY),
            access_token: read(ACCESS_TOKEN_KEY),
            batch_size: read(BATCH_SIZE_KEY),
            lease_ms: read(LEASE_MS_KEY),
            provider_timeout_ms: read(PROVIDER_TIMEOUT_MS_KEY),
            lease_safety_margin_ms: read(LEASE_SAFETY_MARGIN_MS_KEY),
            retry_delay_ms: read(RETRY_DELAY_MS_KEY),
            poll_interval_ms: read(POLL_INTERVAL_MS_KEY),
            max_attempts: read(MAX_ATTEMPTS_KEY),
        }
    }
}

#[derive(Clone)]
pub struct PushConfig {
    endpoint: String,
    access_token: Option<SensitiveValue>,
    batch_size: u32,
    lease_duration: Duration,
    provider_timeout: Duration,
    lease_safety_margin: Duration,
    retry_delay: Duration,
    poll_interval: Duration,
    max_attempts: u32,
}

impl PushConfig {
    pub fn from_env(environment: AppEnvironment) -> Result<Self, ConfigError> {
        Self::resolve(environment, PushConfigInput::from_env())
    }

    pub fn resolve(
        environment: AppEnvironment,
        input: PushConfigInput,
    ) -> Result<Self, ConfigError> {
        let endpoint = endpoint(environment, input.endpoint.as_deref())?;
        let access_token = access_token(input.access_token.as_deref())?;
        let budget = worker_budget(&input)?;

        Ok(Self {
            endpoint,
            access_token,
            batch_size: budget.batch_size,
            lease_duration: budget.lease_duration,
            provider_timeout: budget.provider_timeout,
            lease_safety_margin: budget.lease_safety_margin,
            retry_delay: budget.retry_delay,
            poll_interval: budget.poll_interval,
            max_attempts: budget.max_attempts,
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub(crate) fn access_token(&self) -> Option<&str> {
        self.access_token.as_ref().map(SensitiveValue::expose)
    }

    pub fn batch_size(&self) -> u32 {
        self.batch_size
    }

    pub fn lease_duration(&self) -> Duration {
        self.lease_duration
    }

    pub fn provider_timeout(&self) -> Duration {
        self.provider_timeout
    }

    pub fn lease_safety_margin(&self) -> Duration {
        self.lease_safety_margin
    }

    pub fn retry_delay(&self) -> Duration {
        self.retry_delay
    }

    pub fn poll_interval(&self) -> Duration {
        self.poll_interval
    }

    pub fn max_attempts(&self) -> u32 {
        self.max_attempts
    }
}

impl fmt::Debug for PushConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PushConfig")
            .field("endpoint", &self.endpoint)
            .field("access_token", &self.access_token)
            .field("batch_size", &self.batch_size)
            .field("lease_duration", &self.lease_duration)
            .field("provider_timeout", &self.provider_timeout)
            .field("lease_safety_margin", &self.lease_safety_margin)
            .field("retry_delay", &self.retry_delay)
            .field("poll_interval", &self.poll_interval)
            .field("max_attempts", &self.max_attempts)
            .finish()
    }
}

#[derive(Clone)]
struct SensitiveValue(String);

impl SensitiveValue {
    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SensitiveValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

fn read(key: &'static str) -> Option<String> {
    env::var(key).ok()
}

fn endpoint(environment: AppEnvironment, value: Option<&str>) -> Result<String, ConfigError> {
    let endpoint = value.unwrap_or(EXPO_PUSH_SEND_URL);
    let permitted = valid_endpoint_configuration(endpoint)
        && (environment != AppEnvironment::Production || endpoint == EXPO_PUSH_SEND_URL);
    permitted.then(|| endpoint.to_owned()).ok_or_else(|| {
        ConfigError::new(
            ENDPOINT_KEY,
            "must use the permitted exact Expo send endpoint",
        )
    })
}

fn access_token(value: Option<&str>) -> Result<Option<SensitiveValue>, ConfigError> {
    value
        .map(|access_token| {
            valid_access_token_configuration(access_token)
                .then(|| SensitiveValue(access_token.to_owned()))
                .ok_or_else(|| {
                    ConfigError::new(
                        ACCESS_TOKEN_KEY,
                        "must be non-empty printable ASCII within the provider limit",
                    )
                })
        })
        .transpose()
}

struct WorkerBudget {
    batch_size: u32,
    lease_duration: Duration,
    provider_timeout: Duration,
    lease_safety_margin: Duration,
    retry_delay: Duration,
    poll_interval: Duration,
    max_attempts: u32,
}

fn worker_budget(input: &PushConfigInput) -> Result<WorkerBudget, ConfigError> {
    let (lease_duration, provider_timeout, lease_safety_margin) = lease_budget(input)?;
    Ok(WorkerBudget {
        batch_size: unsigned(
            BATCH_SIZE_KEY,
            input.batch_size.as_deref().unwrap_or(DEFAULT_BATCH_SIZE),
            1,
            500,
        )?,
        lease_duration,
        provider_timeout,
        lease_safety_margin,
        retry_delay: milliseconds(
            RETRY_DELAY_MS_KEY,
            input
                .retry_delay_ms
                .as_deref()
                .unwrap_or(DEFAULT_RETRY_DELAY_MS),
            1,
            300_000,
        )?,
        poll_interval: milliseconds(
            POLL_INTERVAL_MS_KEY,
            input
                .poll_interval_ms
                .as_deref()
                .unwrap_or(DEFAULT_POLL_INTERVAL_MS),
            10,
            60_000,
        )?,
        max_attempts: unsigned(
            MAX_ATTEMPTS_KEY,
            input
                .max_attempts
                .as_deref()
                .unwrap_or(DEFAULT_MAX_ATTEMPTS),
            1,
            100,
        )?,
    })
}

fn lease_budget(input: &PushConfigInput) -> Result<(Duration, Duration, Duration), ConfigError> {
    let lease_duration = milliseconds(
        LEASE_MS_KEY,
        input.lease_ms.as_deref().unwrap_or(DEFAULT_LEASE_MS),
        1_000,
        300_000,
    )?;
    let provider_timeout = milliseconds(
        PROVIDER_TIMEOUT_MS_KEY,
        input
            .provider_timeout_ms
            .as_deref()
            .unwrap_or(DEFAULT_PROVIDER_TIMEOUT_MS),
        100,
        30_000,
    )?;
    let lease_safety_margin = milliseconds(
        LEASE_SAFETY_MARGIN_MS_KEY,
        input
            .lease_safety_margin_ms
            .as_deref()
            .unwrap_or(DEFAULT_LEASE_SAFETY_MARGIN_MS),
        1,
        30_000,
    )?;
    let provider_budget = provider_timeout
        .checked_add(lease_safety_margin)
        .ok_or_else(|| ConfigError::new(LEASE_MS_KEY, "exceeds the supported duration"))?;
    if lease_duration <= provider_budget {
        return Err(ConfigError::new(
            LEASE_MS_KEY,
            "must exceed provider timeout plus safety margin",
        ));
    }
    Ok((lease_duration, provider_timeout, lease_safety_margin))
}

fn unsigned(
    key: &'static str,
    value: &str,
    minimum: u32,
    maximum: u32,
) -> Result<u32, ConfigError> {
    let value = value
        .parse::<u32>()
        .map_err(|_| ConfigError::new(key, "must be an integer"))?;
    if !(minimum..=maximum).contains(&value) {
        return Err(ConfigError::new(key, "is outside the permitted range"));
    }
    Ok(value)
}

fn milliseconds(
    key: &'static str,
    value: &str,
    minimum: u64,
    maximum: u64,
) -> Result<Duration, ConfigError> {
    let value = value
        .parse::<u64>()
        .map_err(|_| ConfigError::new(key, "must be an integer"))?;
    if !(minimum..=maximum).contains(&value) {
        return Err(ConfigError::new(key, "is outside the permitted range"));
    }
    Ok(Duration::from_millis(value))
}
