//! Argument parsing and terminal-safe rendering: no database needed.

use jamye_server::{
    domain::moderation::ReportStatus,
    platform::logging::syslog_line,
    transport::admin::{
        cli::{Command, DEFAULT_LIST_LIMIT, DEFAULT_PURGE_DAYS, Parsed, USAGE, parse},
        render::terminal_safe,
    },
};
use uuid::Uuid;

use crate::TestResult;

fn run(args: &[&str]) -> TestResult<Command> {
    match parse(args.iter().copied())? {
        Parsed::Run(command) => Ok(command),
        Parsed::Help => Err(format!("{args:?} unexpectedly asked for help").into()),
    }
}

fn expect_usage(args: &[&str]) -> TestResult<String> {
    match parse(args.iter().copied()) {
        Err(error) => Ok(error.to_string()),
        Ok(parsed) => Err(format!("{args:?} was accepted as {parsed:?}").into()),
    }
}

#[test]
fn help_is_requested_with_either_flag_anywhere_and_lists_every_command() -> TestResult {
    for args in [
        vec!["--help"],
        vec!["-h"],
        vec!["reports", "--help"],
        vec!["reports", "list", "-h"],
        vec!["users", "suspend", "--help"],
    ] {
        assert_eq!(parse(args.iter().copied())?, Parsed::Help);
    }
    for command in [
        "reports list",
        "reports show",
        "reports dismiss",
        "reports resolve",
        "reports purge",
        "messages delete",
        "users suspend",
        "users unsuspend",
        "accounts delete",
    ] {
        assert!(USAGE.contains(command), "usage does not list `{command}`");
    }
    assert!(USAGE.contains("EXIT STATUS"));
    Ok(())
}

#[test]
fn every_command_parses_with_its_documented_defaults() -> TestResult {
    let id = Uuid::new_v4();
    let text = id.to_string();
    assert_eq!(
        run(&["reports", "list"])?,
        Command::ReportsList {
            status: Some(ReportStatus::Open),
            limit: DEFAULT_LIST_LIMIT,
            json: false
        }
    );
    assert_eq!(
        run(&["reports", "list", "--status", "all", "--limit=7", "--json"])?,
        Command::ReportsList {
            status: None,
            limit: 7,
            json: true
        }
    );
    assert_eq!(
        run(&["reports", "list", "--status=actioned"])?,
        Command::ReportsList {
            status: Some(ReportStatus::Actioned),
            limit: DEFAULT_LIST_LIMIT,
            json: false
        }
    );
    assert_eq!(
        run(&["reports", "show", &text, "--json"])?,
        Command::ReportsShow {
            report_id: id,
            json: true
        }
    );
    assert_eq!(
        run(&["reports", "dismiss", &text])?,
        Command::ReportsDismiss { report_id: id }
    );
    assert_eq!(
        run(&["reports", "resolve", &text])?,
        Command::ReportsResolve { report_id: id }
    );
    assert_eq!(
        DEFAULT_PURGE_DAYS, 365,
        "the handled-report retention policy is one year"
    );
    assert_eq!(
        run(&["reports", "purge"])?,
        Command::ReportsPurge {
            older_than_days: 365
        }
    );
    assert_eq!(
        run(&["reports", "purge", "--older-than-days", "30"])?,
        Command::ReportsPurge {
            older_than_days: 30
        }
    );
    assert_eq!(
        run(&["messages", "delete", &text])?,
        Command::MessagesDelete { message_id: id }
    );
    assert_eq!(
        run(&["users", "suspend", &text])?,
        Command::UsersSuspend {
            user_id: id,
            reason: None
        }
    );
    assert_eq!(
        run(&["users", "suspend", &text, "--reason", "two words"])?,
        Command::UsersSuspend {
            user_id: id,
            reason: Some("two words".to_owned())
        }
    );
    assert_eq!(
        run(&["users", "suspend", "--reason=inline", &text])?,
        Command::UsersSuspend {
            user_id: id,
            reason: Some("inline".to_owned())
        }
    );
    assert_eq!(
        run(&["users", "unsuspend", &text])?,
        Command::UsersUnsuspend { user_id: id }
    );
    assert_eq!(
        run(&["accounts", "delete", &text])?,
        Command::AccountsDelete { user_id: id }
    );
    Ok(())
}

#[test]
fn malformed_invocations_are_usage_errors() -> TestResult {
    let id = Uuid::new_v4().to_string();
    for args in [
        vec![],
        vec!["reports"],
        vec!["reports", "frobnicate"],
        vec!["frobnicate", "list"],
        vec!["--json", "reports", "list"],
        vec!["reports", "list", "extra"],
        vec!["reports", "list", "--status"],
        vec!["reports", "list", "--status", "pending"],
        vec!["reports", "list", "--limit", "0"],
        vec!["reports", "list", "--limit", "201"],
        vec!["reports", "list", "--limit", "ten"],
        vec!["reports", "list", "--json", "--json"],
        vec!["reports", "list", "--json=yes"],
        vec!["reports", "list", "--unknown"],
        vec!["reports", "list", "-x"],
        vec!["reports", "show"],
        vec!["reports", "show", "not-a-uuid"],
        vec!["reports", "show", &id, &id],
        vec!["reports", "dismiss", &id, "--json"],
        vec!["reports", "purge", "--older-than-days", "0"],
        vec!["reports", "purge", "--older-than-days", "3651"],
        vec!["reports", "purge", "--older-than-days", "-1"],
        vec!["reports", "purge", &id],
        vec!["messages", "delete"],
        vec!["messages", "delete", "--report", &id],
        vec!["users", "suspend", &id, "--reason"],
        vec!["users", "suspend", &id, "--reason", "a", "--reason", "b"],
        vec!["users", "unsuspend", &id, "--reason", "x"],
        vec!["accounts", "delete"],
        vec!["accounts", "delete", "123"],
    ] {
        let message = expect_usage(&args)?;
        assert!(!message.is_empty(), "{args:?} gave an empty message");
    }
    Ok(())
}

#[test]
fn text_is_escaped_for_terminals_and_truncated() {
    assert_eq!(terminal_safe("plain 한국어", 100), "plain 한국어");
    assert_eq!(terminal_safe("a\nb\r\tc", 100), "a\\nb\\r\\tc");
    assert_eq!(terminal_safe("x\u{1b}[31my", 100), "x\\u{001b}[31my");
    assert_eq!(
        terminal_safe("a\u{202e}b\u{2066}c", 100),
        "a\\u{202e}b\\u{2066}c"
    );
    assert_eq!(terminal_safe("abcdef", 3), "abc...");
    assert_eq!(terminal_safe("abc", 3), "abc");
}

#[test]
fn audit_lines_are_forwarded_to_syslog_as_authpriv_notice_with_the_admin_tag() {
    assert_eq!(
        syslog_line(b"{\"event_kind\":\"admin_message_deleted\"}\n"),
        "<85>jamye-server-admin: {\"event_kind\":\"admin_message_deleted\"}"
    );
}
