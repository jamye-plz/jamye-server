//! Argument parsing for the `admin` binary. The command set is small and fixed, so the parser
//! is hand-written: no new dependency, and every accepted form is listed in `USAGE`.

use std::fmt;

use uuid::Uuid;

use crate::{
    application::moderation::admin::{MAX_PURGE_RETENTION_DAYS, MAX_REPORT_LIST_LIMIT},
    domain::moderation::ReportStatus,
};

pub const DEFAULT_LIST_LIMIT: u32 = 50;
/// Policy retention for handled reports: one year.
pub const DEFAULT_PURGE_DAYS: u32 = 365;

pub const USAGE: &str = "\
admin: operator commands for jamye-server moderation

USAGE:
    admin <group> <command> [arguments]

COMMANDS:
    reports list [--status open|actioned|dismissed|all] [--limit <n>] [--json]
    reports show <report_id> [--json]
    reports dismiss <report_id>
    reports resolve <report_id>
    reports purge [--older-than-days <n>]
    messages delete <message_id>
    users suspend <user_id> [--reason <text>]
    users unsuspend <user_id>
    accounts delete <user_id>

OPTIONS:
    -h, --help    print this help and exit with status 0

ENVIRONMENT:
    The same variables as the API (JAMYE_ENVIRONMENT, DATABASE_URL, ...). Only PostgreSQL is used.

EXIT STATUS:
    0 success, 1 operational failure (unknown id, database error), 2 usage error
";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    ReportsList {
        /// `None` lists every status.
        status: Option<ReportStatus>,
        limit: u32,
        json: bool,
    },
    ReportsShow {
        report_id: Uuid,
        json: bool,
    },
    ReportsDismiss {
        report_id: Uuid,
    },
    ReportsResolve {
        report_id: Uuid,
    },
    ReportsPurge {
        older_than_days: u32,
    },
    MessagesDelete {
        message_id: Uuid,
    },
    UsersSuspend {
        user_id: Uuid,
        reason: Option<String>,
    },
    UsersUnsuspend {
        user_id: Uuid,
    },
    AccountsDelete {
        user_id: Uuid,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Parsed {
    Help,
    Run(Command),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsageError(String);

impl UsageError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UsageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for UsageError {}

/// Parses the arguments after the program name.
pub fn parse<I, S>(args: I) -> Result<Parsed, UsageError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args: Vec<String> = args.into_iter().map(Into::into).collect();
    if args
        .iter()
        .any(|argument| argument == "-h" || argument == "--help")
    {
        return Ok(Parsed::Help);
    }
    let (group, action, rest) = match args.as_slice() {
        [] => return Err(UsageError::new("a command is required")),
        [group] => {
            return Err(UsageError::new(format!("`{group}` needs a command")));
        }
        [group, action, rest @ ..] => (group.as_str(), action.as_str(), rest),
    };
    let command = match (group, action) {
        ("reports", "list") => parse_reports_list(rest)?,
        ("reports", "show") => {
            let scanned = scan(rest, &[], &["--json"])?;
            Command::ReportsShow {
                report_id: single_id(&scanned, "report_id")?,
                json: scanned.has("--json"),
            }
        }
        ("reports", "dismiss") => Command::ReportsDismiss {
            report_id: single_id(&scan(rest, &[], &[])?, "report_id")?,
        },
        ("reports", "resolve") => Command::ReportsResolve {
            report_id: single_id(&scan(rest, &[], &[])?, "report_id")?,
        },
        ("reports", "purge") => parse_reports_purge(rest)?,
        ("messages", "delete") => Command::MessagesDelete {
            message_id: single_id(&scan(rest, &[], &[])?, "message_id")?,
        },
        ("users", "suspend") => {
            let scanned = scan(rest, &["--reason"], &[])?;
            Command::UsersSuspend {
                user_id: single_id(&scanned, "user_id")?,
                reason: scanned.value("--reason").map(str::to_owned),
            }
        }
        ("users", "unsuspend") => Command::UsersUnsuspend {
            user_id: single_id(&scan(rest, &[], &[])?, "user_id")?,
        },
        ("accounts", "delete") => Command::AccountsDelete {
            user_id: single_id(&scan(rest, &[], &[])?, "user_id")?,
        },
        _ => {
            return Err(UsageError::new(format!(
                "unknown command `{group} {action}`"
            )));
        }
    };
    Ok(Parsed::Run(command))
}

fn parse_reports_list(rest: &[String]) -> Result<Command, UsageError> {
    let scanned = scan(rest, &["--status", "--limit"], &["--json"])?;
    no_positional(&scanned)?;
    let status =
        match scanned.value("--status") {
            None | Some("open") => Some(ReportStatus::Open),
            Some("all") => None,
            Some(other) => Some(ReportStatus::parse(other).ok_or_else(|| {
                UsageError::new("--status must be open, actioned, dismissed or all")
            })?),
        };
    let limit = match scanned.value("--limit") {
        None => DEFAULT_LIST_LIMIT,
        Some(value) => bounded_number("--limit", value, MAX_REPORT_LIST_LIMIT)?,
    };
    Ok(Command::ReportsList {
        status,
        limit,
        json: scanned.has("--json"),
    })
}

fn parse_reports_purge(rest: &[String]) -> Result<Command, UsageError> {
    let scanned = scan(rest, &["--older-than-days"], &[])?;
    no_positional(&scanned)?;
    let older_than_days = match scanned.value("--older-than-days") {
        None => DEFAULT_PURGE_DAYS,
        Some(value) => bounded_number("--older-than-days", value, MAX_PURGE_RETENTION_DAYS)?,
    };
    Ok(Command::ReportsPurge { older_than_days })
}

fn bounded_number(option: &str, value: &str, maximum: u32) -> Result<u32, UsageError> {
    match value.parse::<u32>() {
        Ok(number) if (1..=maximum).contains(&number) => Ok(number),
        _ => Err(UsageError::new(format!(
            "{option} must be a whole number from 1 to {maximum}"
        ))),
    }
}

#[derive(Default)]
struct Scanned {
    positional: Vec<String>,
    options: Vec<(String, String)>,
    switches: Vec<String>,
}

impl Scanned {
    fn value(&self, option: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|(name, _)| name == option)
            .map(|(_, value)| value.as_str())
    }

    fn has(&self, switch: &str) -> bool {
        self.switches.iter().any(|name| name == switch)
    }
}

/// Splits `args` into positionals, `--name value` / `--name=value` options and `--switch`es.
/// Anything starting with `-` that is not declared is a usage error.
fn scan(args: &[String], value_options: &[&str], switches: &[&str]) -> Result<Scanned, UsageError> {
    let mut scanned = Scanned::default();
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        index += 1;
        if let Some(long) = argument.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((name, value)) => (name, Some(value.to_owned())),
                None => (long, None),
            };
            let flag = format!("--{name}");
            if switches.contains(&flag.as_str()) {
                if inline.is_some() {
                    return Err(UsageError::new(format!("{flag} does not take a value")));
                }
                if scanned.has(&flag) {
                    return Err(UsageError::new(format!("{flag} was given twice")));
                }
                scanned.switches.push(flag);
            } else if value_options.contains(&flag.as_str()) {
                let value = match inline {
                    Some(value) => value,
                    None => {
                        let value = args
                            .get(index)
                            .ok_or_else(|| UsageError::new(format!("{flag} needs a value")))?;
                        index += 1;
                        value.clone()
                    }
                };
                if scanned.value(&flag).is_some() {
                    return Err(UsageError::new(format!("{flag} was given twice")));
                }
                scanned.options.push((flag, value));
            } else {
                return Err(UsageError::new(format!("unknown option {flag}")));
            }
        } else if argument.starts_with('-') && argument.len() > 1 {
            return Err(UsageError::new(format!("unknown option {argument}")));
        } else {
            scanned.positional.push(argument.clone());
        }
    }
    Ok(scanned)
}

fn no_positional(scanned: &Scanned) -> Result<(), UsageError> {
    match scanned.positional.first() {
        Some(extra) => Err(UsageError::new(format!("unexpected argument `{extra}`"))),
        None => Ok(()),
    }
}

fn single_id(scanned: &Scanned, name: &str) -> Result<Uuid, UsageError> {
    match scanned.positional.as_slice() {
        [] => Err(UsageError::new(format!("<{name}> is required"))),
        [value] => Uuid::try_parse(value)
            .map_err(|_| UsageError::new(format!("<{name}> must be a UUID, got `{value}`"))),
        [_, extra, ..] => Err(UsageError::new(format!("unexpected argument `{extra}`"))),
    }
}
