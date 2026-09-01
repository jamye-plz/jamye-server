//! Cleanup-only credentials and worker timing configuration.

use std::{env, fmt, time::Duration};

use crate::{
    adapters::object_storage::account_deletion::S3AccountObjectDeletionCredentials,
    application::account_deletion::cleanup::AccountObjectDeletionWorkerConfig, config::ConfigError,
};

const ACCESS_KEY: &str = "JAMYE_ACCOUNT_OBJECT_DELETION_ACCESS_KEY_ID";
const SECRET_KEY: &str = "JAMYE_ACCOUNT_OBJECT_DELETION_SECRET_ACCESS_KEY";

#[derive(Clone)]
pub struct AccountDeletionConfig {
    credentials: S3AccountObjectDeletionCredentials,
    worker: AccountObjectDeletionWorkerConfig,
}

#[derive(Clone, Default)]
pub struct AccountDeletionConfigInput {
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub batch_size: Option<String>,
    pub lease_ms: Option<String>,
    pub delete_timeout_ms: Option<String>,
    pub lease_safety_margin_ms: Option<String>,
    pub retry_delay_ms: Option<String>,
    pub poll_interval_ms: Option<String>,
    pub max_attempts: Option<String>,
}

impl AccountDeletionConfigInput {
    pub fn from_env() -> Self {
        Self {
            access_key_id: env::var(ACCESS_KEY).ok(),
            secret_access_key: env::var(SECRET_KEY).ok(),
            batch_size: env::var("JAMYE_ACCOUNT_OBJECT_DELETION_BATCH_SIZE").ok(),
            lease_ms: env::var("JAMYE_ACCOUNT_OBJECT_DELETION_LEASE_MS").ok(),
            delete_timeout_ms: env::var("JAMYE_ACCOUNT_OBJECT_DELETION_DELETE_TIMEOUT_MS").ok(),
            lease_safety_margin_ms: env::var(
                "JAMYE_ACCOUNT_OBJECT_DELETION_LEASE_SAFETY_MARGIN_MS",
            )
            .ok(),
            retry_delay_ms: env::var("JAMYE_ACCOUNT_OBJECT_DELETION_RETRY_DELAY_MS").ok(),
            poll_interval_ms: env::var("JAMYE_ACCOUNT_OBJECT_DELETION_POLL_INTERVAL_MS").ok(),
            max_attempts: env::var("JAMYE_ACCOUNT_OBJECT_DELETION_MAX_ATTEMPTS").ok(),
        }
    }
}

impl AccountDeletionConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::resolve(AccountDeletionConfigInput::from_env())
    }

    pub fn resolve(input: AccountDeletionConfigInput) -> Result<Self, ConfigError> {
        let access_key_id = required(ACCESS_KEY, input.access_key_id)?;
        let secret_access_key = required(SECRET_KEY, input.secret_access_key)?;
        let credentials = S3AccountObjectDeletionCredentials::new(access_key_id, secret_access_key)
            .map_err(|error| match error {
                crate::adapters::object_storage::account_deletion::S3AccountObjectDeletionConfigError::InvalidAccessKeyId => ConfigError::new(ACCESS_KEY, "uses an invalid cleanup credential"),
                crate::adapters::object_storage::account_deletion::S3AccountObjectDeletionConfigError::InvalidSecretAccessKey => ConfigError::new(SECRET_KEY, "uses an invalid cleanup credential"),
                crate::adapters::object_storage::account_deletion::S3AccountObjectDeletionConfigError::ReusesMediaCredentials => ConfigError::new(ACCESS_KEY, "must use a dedicated cleanup identity"),
            })?;
        let worker = AccountObjectDeletionWorkerConfig {
            claim_owner: format!("account-cleanup-{}", uuid::Uuid::new_v4()),
            batch_size: positive_u32(
                "JAMYE_ACCOUNT_OBJECT_DELETION_BATCH_SIZE",
                input.batch_size,
                20,
            )?,
            lease_duration: milliseconds(
                "JAMYE_ACCOUNT_OBJECT_DELETION_LEASE_MS",
                input.lease_ms,
                15_000,
            )?,
            delete_timeout: milliseconds(
                "JAMYE_ACCOUNT_OBJECT_DELETION_DELETE_TIMEOUT_MS",
                input.delete_timeout_ms,
                2_000,
            )?,
            lease_safety_margin: milliseconds(
                "JAMYE_ACCOUNT_OBJECT_DELETION_LEASE_SAFETY_MARGIN_MS",
                input.lease_safety_margin_ms,
                1_000,
            )?,
            retry_delay: milliseconds(
                "JAMYE_ACCOUNT_OBJECT_DELETION_RETRY_DELAY_MS",
                input.retry_delay_ms,
                1_000,
            )?,
            poll_interval: milliseconds(
                "JAMYE_ACCOUNT_OBJECT_DELETION_POLL_INTERVAL_MS",
                input.poll_interval_ms,
                250,
            )?,
            max_attempts: positive_u32(
                "JAMYE_ACCOUNT_OBJECT_DELETION_MAX_ATTEMPTS",
                input.max_attempts,
                8,
            )?,
        };
        let budget = worker
            .delete_timeout
            .checked_add(worker.lease_safety_margin)
            .ok_or_else(|| {
                ConfigError::new("JAMYE_ACCOUNT_OBJECT_DELETION_LEASE_MS", "is invalid")
            })?;
        if worker.lease_duration <= budget {
            return Err(ConfigError::new(
                "JAMYE_ACCOUNT_OBJECT_DELETION_LEASE_MS",
                "must exceed delete timeout plus safety margin",
            ));
        }
        Ok(Self {
            credentials,
            worker,
        })
    }

    pub(crate) fn credentials(&self) -> &S3AccountObjectDeletionCredentials {
        &self.credentials
    }
    pub(crate) fn worker_config(&self) -> AccountObjectDeletionWorkerConfig {
        self.worker.clone()
    }
}

impl fmt::Debug for AccountDeletionConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccountDeletionConfig")
            .field("credentials", &self.credentials)
            .field("worker", &self.worker)
            .finish()
    }
}

fn required(key: &'static str, value: Option<String>) -> Result<String, ConfigError> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ConfigError::new(key, "is required"))
}

fn positive_u32(
    key: &'static str,
    value: Option<String>,
    default: u32,
) -> Result<u32, ConfigError> {
    match value {
        Some(value) => value
            .parse::<u32>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| ConfigError::new(key, "must be a positive integer")),
        None => Ok(default),
    }
}

fn milliseconds(
    key: &'static str,
    value: Option<String>,
    default: u64,
) -> Result<Duration, ConfigError> {
    match value {
        Some(value) => value
            .parse::<u64>()
            .ok()
            .filter(|value| *value > 0)
            .map(Duration::from_millis)
            .ok_or_else(|| ConfigError::new(key, "must be positive milliseconds")),
        None => Ok(Duration::from_millis(default)),
    }
}
