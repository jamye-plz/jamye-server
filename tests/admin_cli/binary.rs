//! The real `admin` binary: exit codes, streams, structured audit lines and no listener.

use std::{
    net::TcpListener,
    process::{Command, Output},
};

use serde_json::Value;
use uuid::Uuid;

use crate::{
    TestResult,
    harness::AdminHarness,
    support::{insert_group, insert_message, insert_user},
};

fn admin(database_url: Option<&str>) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_admin"));
    command.env_clear().env("JAMYE_ENVIRONMENT", "test");
    if let Some(url) = database_url {
        command.env("DATABASE_URL", url);
    }
    command
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The structured log lines the binary wrote (stderr lines that parse as JSON objects).
fn audit_lines(output: &Output) -> Vec<Value> {
    stderr(output)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(Value::is_object)
        .collect()
}

#[test]
fn help_exits_0_and_usage_errors_exit_2_without_touching_the_database() -> TestResult {
    // No DATABASE_URL at all: parsing happens before any configuration or connection.
    let help = admin(None).arg("--help").output()?;
    assert_eq!(help.status.code(), Some(0));
    assert!(stdout(&help).contains("reports purge"));
    assert!(stdout(&help).contains("accounts delete"));

    let none = admin(None).output()?;
    assert_eq!(none.status.code(), Some(2));
    assert!(stdout(&none).is_empty());
    assert!(stderr(&none).contains("USAGE"));

    for args in [
        vec!["reports", "frobnicate"],
        vec!["reports", "show", "not-a-uuid"],
        vec!["reports", "list", "--status", "pending"],
        vec!["users", "suspend"],
    ] {
        let output = admin(None).args(&args).output()?;
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(stderr(&output).contains("error:"), "{args:?}");
    }
    Ok(())
}

#[test]
fn a_missing_database_setting_is_an_operational_failure() -> TestResult {
    let output = admin(None).args(["reports", "list"]).output()?;
    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("DATABASE_URL"), "{message}");
    Ok(())
}

#[tokio::test]
async fn the_binary_runs_commands_prints_to_stdout_and_logs_audit_lines_to_stderr() -> TestResult {
    let harness = AdminHarness::new().await?;
    let result: TestResult = async {
        let url = harness.database.database_url().to_owned();
        let author = insert_user(&harness.pool, "author").await?;
        let other = insert_user(&harness.pool, "other").await?;
        let group = insert_group(&harness.pool, &[author, other]).await?;
        let message = insert_message(&harness.pool, group.chatroom_id, author, "words").await?;

        // The server listen address is held by this test: the admin must never bind it.
        let held = TcpListener::bind("127.0.0.1:0")?;
        let held_address = held.local_addr()?.to_string();

        let listed = admin(Some(&url))
            .env("JAMYE_LISTEN_ADDR", &held_address)
            .args(["reports", "list"])
            .output()?;
        assert_eq!(listed.status.code(), Some(0), "{}", stderr(&listed));
        assert_eq!(stdout(&listed), "no reports\n");

        let json = admin(Some(&url))
            .args(["reports", "list", "--json"])
            .output()?;
        assert_eq!(json.status.code(), Some(0));
        assert_eq!(
            serde_json::from_str::<Value>(&stdout(&json))?,
            serde_json::json!([])
        );

        // A mutation: human text on stdout, one structured audit line on stderr.
        let suspended = admin(Some(&url))
            .args([
                "users",
                "suspend",
                &author.to_string(),
                "--reason",
                "reason-marker-7f3a",
            ])
            .output()?;
        assert_eq!(suspended.status.code(), Some(0), "{}", stderr(&suspended));
        assert!(stdout(&suspended).contains("suspended"));
        let lines = audit_lines(&suspended);
        let line = lines
            .iter()
            .find(|line| line["event_kind"] == "account_suspended")
            .ok_or_else(|| format!("no audit line in {}", stderr(&suspended)))?;
        assert_eq!(line["user_id"], author.to_string());
        assert!(line["timestamp"].is_string(), "{line}");
        assert!(
            !stderr(&suspended).contains("reason-marker-7f3a"),
            "the reason text stays out of the audit line"
        );

        let deleted = admin(Some(&url))
            .args(["messages", "delete", &message.to_string()])
            .output()?;
        assert_eq!(deleted.status.code(), Some(0), "{}", stderr(&deleted));
        let lines = audit_lines(&deleted);
        let line = lines
            .iter()
            .find(|line| line["event_kind"] == "admin_message_deleted")
            .ok_or_else(|| format!("no audit line in {}", stderr(&deleted)))?;
        assert_eq!(line["message_id"], message.to_string());

        // Operational failures: exit 1 with the reason on stderr and nothing on stdout.
        for args in [
            vec![
                "users".to_owned(),
                "suspend".to_owned(),
                Uuid::new_v4().to_string(),
            ],
            vec![
                "messages".to_owned(),
                "delete".to_owned(),
                Uuid::new_v4().to_string(),
            ],
            vec![
                "reports".to_owned(),
                "dismiss".to_owned(),
                Uuid::new_v4().to_string(),
            ],
            vec![
                "accounts".to_owned(),
                "delete".to_owned(),
                Uuid::new_v4().to_string(),
            ],
        ] {
            let output = admin(Some(&url)).args(&args).output()?;
            assert_eq!(
                output.status.code(),
                Some(1),
                "{args:?}: {}",
                stderr(&output)
            );
            assert!(stdout(&output).is_empty(), "{args:?}");
            assert!(stderr(&output).contains("not found"), "{args:?}");
        }
        drop(held);
        Ok(())
    }
    .await;
    harness.finish(result).await
}
