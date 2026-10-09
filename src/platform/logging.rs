//! Structured stdout logging without headers, bodies, or credential values.

use std::{env, error::Error, fmt, io, os::unix::net::UnixDatagram};

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

/// Installs the JSON subscriber for the admin CLI. Its stdout carries the command output, so
/// the structured audit lines go to stderr, and a copy goes to the local syslog socket (the
/// systemd journal on the production host) so the audit trail outlives the terminal session.
pub fn init_json_logging_audit() -> Result<(), LoggingInitError> {
    let filter = filter_from_environment()?;
    let subscriber = json_subscriber(StderrAndSyslog, filter);
    subscriber
        .try_init()
        .map_err(|_| LoggingInitError::SubscriberAlreadyInstalled)
}

const SYSLOG_SOCKET: &str = "/dev/log";
const SYSLOG_TAG: &str = "jamye-server-admin";
/// `<facility * 8 + severity>`: authpriv (10), notice (5).
const SYSLOG_PRIORITY: &str = "<85>";

/// The datagram sent to syslog for one structured log line.
pub fn syslog_line(line: &[u8]) -> String {
    format!(
        "{SYSLOG_PRIORITY}{SYSLOG_TAG}: {}",
        String::from_utf8_lossy(line).trim_end()
    )
}

#[derive(Clone, Copy, Debug)]
struct StderrAndSyslog;

struct StderrAndSyslogWriter;

impl<'writer> MakeWriter<'writer> for StderrAndSyslog {
    type Writer = StderrAndSyslogWriter;

    fn make_writer(&'writer self) -> Self::Writer {
        StderrAndSyslogWriter
    }
}

impl io::Write for StderrAndSyslogWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        io::stderr().write_all(buffer)?;
        // Best effort: a missing or unwritable syslog socket never fails a command.
        if let Ok(socket) = UnixDatagram::unbound() {
            let _ = socket.send_to(syslog_line(buffer).as_bytes(), SYSLOG_SOCKET);
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        io::stderr().flush()
    }
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
