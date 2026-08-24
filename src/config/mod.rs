//! Environment-backed process configuration.

use std::{env, error::Error, fmt, net::SocketAddr, time::Duration};

use url::Url;

const DEFAULT_LISTEN_ADDRESS: &str = "127.0.0.1:3000";
const DEFAULT_SHUTDOWN_GRACE_SECONDS: &str = "20";
const DEFAULT_READINESS_TIMEOUT_MS: &str = "1000";

/// Deployment environment used only for safe behavior selection and logging.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppEnvironment {
    Development,
    Test,
    Production,
}

/// Unvalidated configuration values.
///
/// `AppConfig::from_env` is the production source. This value object keeps
/// validation deterministic and independently testable without mutating the
/// process environment.
#[derive(Clone, Default)]
pub struct ConfigInput {
    pub environment: Option<String>,
    pub listen_address: Option<String>,
    pub shutdown_grace_seconds: Option<String>,
    pub readiness_timeout_ms: Option<String>,
    pub database_url: Option<String>,
    pub redis_url: Option<String>,
    pub minio_health_url: Option<String>,
}

impl ConfigInput {
    fn from_env() -> Self {
        Self {
            environment: read_env("JAMYE_ENVIRONMENT"),
            listen_address: read_env("JAMYE_LISTEN_ADDR"),
            shutdown_grace_seconds: read_env("JAMYE_SHUTDOWN_GRACE_SECONDS"),
            readiness_timeout_ms: read_env("JAMYE_READINESS_TIMEOUT_MS"),
            database_url: read_env("DATABASE_URL"),
            redis_url: read_env("REDIS_URL"),
            minio_health_url: read_env("JAMYE_MINIO_HEALTH_URL"),
        }
    }
}

/// Validated process configuration.
#[derive(Clone, Debug)]
pub struct AppConfig {
    environment: AppEnvironment,
    listen_address: SocketAddr,
    shutdown_grace: Duration,
    readiness_timeout: Duration,
    database_url: SensitiveUrl,
    redis_url: Option<SensitiveUrl>,
    minio_health_url: Option<SensitiveUrl>,
}

impl AppConfig {
    /// Loads and validates the current process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::try_from(ConfigInput::from_env())
    }

    pub fn environment(&self) -> AppEnvironment {
        self.environment
    }

    pub fn listen_address(&self) -> SocketAddr {
        self.listen_address
    }

    pub fn shutdown_grace(&self) -> Duration {
        self.shutdown_grace
    }

    pub fn readiness_timeout(&self) -> Duration {
        self.readiness_timeout
    }

    pub(crate) fn database_url(&self) -> &str {
        self.database_url.expose_secret()
    }

    pub(crate) fn redis_url(&self) -> Option<&str> {
        self.redis_url.as_ref().map(SensitiveUrl::expose_secret)
    }

    pub(crate) fn minio_health_url(&self) -> Option<&str> {
        self.minio_health_url
            .as_ref()
            .map(SensitiveUrl::expose_secret)
    }
}

impl TryFrom<ConfigInput> for AppConfig {
    type Error = ConfigError;

    fn try_from(input: ConfigInput) -> Result<Self, Self::Error> {
        let environment = parse_environment(required(
            "JAMYE_ENVIRONMENT",
            input.environment,
        )?)?;
        let listen_address = parse_socket_address(
            "JAMYE_LISTEN_ADDR",
            input.listen_address.as_deref().unwrap_or(DEFAULT_LISTEN_ADDRESS),
        )?;
        let shutdown_grace = parse_duration(
            "JAMYE_SHUTDOWN_GRACE_SECONDS",
            input
                .shutdown_grace_seconds
                .as_deref()
                .unwrap_or(DEFAULT_SHUTDOWN_GRACE_SECONDS),
            DurationUnit::Seconds,
            1,
            300,
        )?;
        let readiness_timeout = parse_duration(
            "JAMYE_READINESS_TIMEOUT_MS",
            input
                .readiness_timeout_ms
                .as_deref()
                .unwrap_or(DEFAULT_READINESS_TIMEOUT_MS),
            DurationUnit::Milliseconds,
            50,
            30_000,
        )?;
        let database_url = validate_url(
            "DATABASE_URL",
            required("DATABASE_URL", input.database_url)?,
            &["postgres", "postgresql"],
            UrlPolicy::SecretService,
        )?;
        let redis_url = optional_url(
            "REDIS_URL",
            input.redis_url,
            &["redis"],
            UrlPolicy::SecretService,
        )?;
        let minio_health_url = optional_url(
            "JAMYE_MINIO_HEALTH_URL",
            input.minio_health_url,
            &["http", "https"],
            UrlPolicy::MinioHealth,
        )?;

        Ok(Self {
            environment,
            listen_address,
            shutdown_grace,
            readiness_timeout,
            database_url,
            redis_url,
            minio_health_url,
        })
    }
}

#[derive(Clone)]
struct SensitiveUrl(String);

impl SensitiveUrl {
    fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SensitiveUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

/// Configuration error that identifies a key without echoing its value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigError {
    key: &'static str,
    reason: &'static str,
}

impl ConfigError {
    fn new(key: &'static str, reason: &'static str) -> Self {
        Self { key, reason }
    }

    pub fn key(&self) -> &'static str {
        self.key
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid configuration for {}: {}",
            self.key, self.reason
        )
    }
}

impl Error for ConfigError {}

#[derive(Clone, Copy)]
enum DurationUnit {
    Milliseconds,
    Seconds,
}

#[derive(Clone, Copy)]
enum UrlPolicy {
    SecretService,
    MinioHealth,
}

fn read_env(key: &'static str) -> Option<String> {
    env::var(key).ok()
}

fn required(key: &'static str, value: Option<String>) -> Result<String, ConfigError> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ConfigError::new(key, "is required"))
}

fn parse_environment(value: String) -> Result<AppEnvironment, ConfigError> {
    match value.as_str() {
        "development" => Ok(AppEnvironment::Development),
        "test" => Ok(AppEnvironment::Test),
        "production" => Ok(AppEnvironment::Production),
        _ => Err(ConfigError::new(
            "JAMYE_ENVIRONMENT",
            "must be development, test, or production",
        )),
    }
}

fn parse_socket_address(key: &'static str, value: &str) -> Result<SocketAddr, ConfigError> {
    value
        .parse()
        .map_err(|_| ConfigError::new(key, "must be an IP socket address"))
}

fn parse_duration(
    key: &'static str,
    value: &str,
    unit: DurationUnit,
    minimum: u64,
    maximum: u64,
) -> Result<Duration, ConfigError> {
    let amount = value
        .parse::<u64>()
        .map_err(|_| ConfigError::new(key, "must be an integer"))?;
    if !(minimum..=maximum).contains(&amount) {
        return Err(ConfigError::new(key, "is outside the permitted range"));
    }
    Ok(match unit {
        DurationUnit::Milliseconds => Duration::from_millis(amount),
        DurationUnit::Seconds => Duration::from_secs(amount),
    })
}

fn optional_url(
    key: &'static str,
    value: Option<String>,
    schemes: &[&str],
    policy: UrlPolicy,
) -> Result<Option<SensitiveUrl>, ConfigError> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(|value| validate_url(key, value, schemes, policy))
        .transpose()
}

fn validate_url(
    key: &'static str,
    value: String,
    schemes: &[&str],
    policy: UrlPolicy,
) -> Result<SensitiveUrl, ConfigError> {
    let parsed = Url::parse(&value).map_err(|_| ConfigError::new(key, "must be a valid URL"))?;
    if !schemes.contains(&parsed.scheme()) || parsed.host_str().is_none() {
        return Err(ConfigError::new(key, "uses an unsupported URL form"));
    }
    if matches!(policy, UrlPolicy::MinioHealth) {
        validate_minio_health_url(key, &parsed)?;
    }
    Ok(SensitiveUrl(value))
}

fn validate_minio_health_url(key: &'static str, url: &Url) -> Result<(), ConfigError> {
    let has_user_info = !url.username().is_empty() || url.password().is_some();
    let has_suffix_data = url.query().is_some() || url.fragment().is_some();
    if has_user_info || has_suffix_data {
        return Err(ConfigError::new(
            key,
            "must not contain credentials, query, or fragment",
        ));
    }
    if url.path().trim_end_matches('/') != "/minio/health/live" {
        return Err(ConfigError::new(
            key,
            "must target /minio/health/live",
        ));
    }
    Ok(())
}
