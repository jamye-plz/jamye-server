use std::{io::Write, process::ExitCode};

use jamye_server::{
    config::AppConfig,
    platform::logging::init_json_logging_audit,
    transport::admin::{
        cli::{Parsed, USAGE, parse},
        runtime::{EXIT_FAILURE, EXIT_USAGE, compose, execute},
    },
};

/// Operator commands over SSH. Human output goes to stdout; structured audit log lines go to
/// stderr. Exit status: 0 success, 1 operational failure, 2 usage error. No listener is opened.
#[tokio::main]
async fn main() -> ExitCode {
    let command = match parse(
        std::env::args_os()
            .skip(1)
            .map(|argument| argument.to_string_lossy().into_owned()),
    ) {
        Ok(Parsed::Help) => {
            return finish(USAGE.as_bytes());
        }
        Ok(Parsed::Run(command)) => command,
        Err(error) => {
            eprintln!("error: {error}\n\n{USAGE}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let config = match AppConfig::from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    if let Err(error) = init_json_logging_audit() {
        eprintln!("error: {error}");
        return ExitCode::from(EXIT_FAILURE);
    }
    let services = match compose(&config) {
        Ok(services) => services,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    match execute(&services, command).await {
        Ok(output) => finish(output.as_bytes()),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(error.exit_code())
        }
    }
}

fn finish(output: &[u8]) -> ExitCode {
    if std::io::stdout().write_all(output).is_err() {
        return ExitCode::from(EXIT_FAILURE);
    }
    ExitCode::SUCCESS
}
