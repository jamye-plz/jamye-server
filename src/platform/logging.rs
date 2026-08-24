//! Structured stdout logging without headers, bodies, or credential values.

use std::{env, error::Error, fmt};

use tracing::Subscriber;
use tracing_subscriber::{EnvFilter, fmt::MakeWriter, util::SubscriberInitExt};

const DEFAULT_FILTER: &str = "jamye_server=info,tower_http=info";

/// Installs the process-wide JSON tracing subscriber.
pub fn init_json_logging() -> Result<(), LoggingInitError> {
    let filter = filter_from_environment()?;
    let subscriber = json_subscriber(std::io::stdout, filter);
    subscriber
        .try_init()
        .map_err(|_| LoggingInitError::SubscriberAlreadyInstalled)
}

/// Builds the same JSON subscriber with a caller-supplied writer for testing.
pub fn build_json_subscriber<W>(
    writer: W,
    filter: &str,
) -> Result<impl Subscriber + Send + Sync + 'static, LoggingInitError>
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    let filter = EnvFilter::try_new(filter).map_err(|_| LoggingInitError::InvalidFilter)?;
    Ok(json_subscriber(writer, filter))
}

fn json_subscriber<W>(writer: W, filter: EnvFilter) -> impl Subscriber + Send + Sync + 'static
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .json()
        .flatten_event(true)
        .with_current_span(true)
        .with_span_list(false)
        .with_target(true)
        .finish()
}

/// Validates a prospective `RUST_LOG` directive without installing it.
pub fn validate_filter(value: &str) -> Result<(), LoggingInitError> {
    EnvFilter::try_new(value)
        .map(|_| ())
        .map_err(|_| LoggingInitError::InvalidFilter)
}

fn filter_from_environment() -> Result<EnvFilter, LoggingInitError> {
    match env::var("RUST_LOG") {
        Ok(value) => {
            validate_filter(&value)?;
            EnvFilter::try_new(value).map_err(|_| LoggingInitError::InvalidFilter)
        }
        Err(env::VarError::NotPresent) => {
            EnvFilter::try_new(DEFAULT_FILTER).map_err(|_| LoggingInitError::InvalidFilter)
        }
        Err(env::VarError::NotUnicode(_)) => Err(LoggingInitError::InvalidFilter),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoggingInitError {
    InvalidFilter,
    SubscriberAlreadyInstalled,
}

impl fmt::Display for LoggingInitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFilter => formatter.write_str("RUST_LOG is not a valid tracing filter"),
            Self::SubscriberAlreadyInstalled => {
                formatter.write_str("the global tracing subscriber is already installed")
            }
        }
    }
}

impl Error for LoggingInitError {}
