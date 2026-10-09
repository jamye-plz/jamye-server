//! `reports list|show|dismiss|resolve|purge`.

use serde_json::Value;
use uuid::Uuid;

use crate::{
    TestResult,
    harness::{
        AdminHarness, ReportFixture, assert_failure, assert_usage, report_exists, report_status,
    },
    support::{count, insert_group, insert_installation, insert_message, insert_user},
};

struct Scenario {
    reporter: Uuid,
    author: Uuid,
    group: Uuid,
    message: Uuid,
}

async fn scenario(harness: &AdminHarness) -> TestResult<Scenario> {
    let reporter = insert_user(&harness.pool, "reporter-nickname").await?;
    let author = insert_user(&harness.pool, "author-nickname").await?;
    let fixture = insert_group(&harness.pool, &[reporter, author]).await?;
    let message = insert_message(&harness.pool, fixture.chatroom_id, author, "stored text").await?;
    Ok(Scenario {
        reporter,
        author,
        group: fixture.group_id,
        message,
    })
}

#[tokio::test]
async fn list_filters_by_status_and_prints_ids_reasons_timestamps_and_snapshot_text() -> TestResult
{
    let harness = AdminHarness::new().await?;
    let result: TestResult = async {
        let s = scenario(&harness).await?;
        let open_message =
            ReportFixture::message(s.reporter, s.group, s.message, "bad *** text\nsecond line")
                .insert(&harness.pool)
                .await?;
        let open_user = ReportFixture::user(s.reporter, s.group, s.author)
            .insert(&harness.pool)
            .await?;
        let dismissed = ReportFixture::user(s.reporter, s.group, s.author)
            .handled("dismissed", 2)
            .insert(&harness.pool)
            .await?;
        let actioned = ReportFixture::message(s.reporter, s.group, s.message, "old text")
            .handled("actioned", 3)
            .insert(&harness.pool)
            .await?;

        // The default is the open queue.
        let open = harness.run(&["reports", "list"]).await?;
        assert!(open.contains(&open_message.to_string()));
        assert!(open.contains(&open_user.to_string()));
        assert!(!open.contains(&dismissed.to_string()));
        assert!(!open.contains(&actioned.to_string()));
        assert!(open.contains("2 report(s)"));
        assert!(open.contains("harassment"), "reason missing: {open}");
        assert!(open.contains(&format!("message {}", s.message)));
        assert!(open.contains(&format!("user {}", s.author)));
        assert!(open.contains(&s.group.to_string()));
        assert!(open.contains(&s.reporter.to_string()));
        // RFC 3339 timestamp, and the snapshot text with the newline escaped.
        assert!(
            open.contains('T') && open.contains('Z'),
            "timestamp missing: {open}"
        );
        assert!(
            open.contains("text: \"bad *** text\\nsecond line\""),
            "snapshot text missing: {open}"
        );
        // Never profile data.
        assert!(!open.contains("nickname"), "list leaked a nickname: {open}");

        let explicit_open = harness
            .run(&["reports", "list", "--status", "open"])
            .await?;
        assert_eq!(explicit_open.matches(" open ").count(), 2);
        let only_dismissed = harness
            .run(&["reports", "list", "--status=dismissed"])
            .await?;
        assert!(only_dismissed.contains(&dismissed.to_string()));
        assert!(only_dismissed.contains("1 report(s)"));
        let only_actioned = harness
            .run(&["reports", "list", "--status", "actioned"])
            .await?;
        assert!(only_actioned.contains(&actioned.to_string()));
        assert!(only_actioned.contains("1 report(s)"));
        let all = harness.run(&["reports", "list", "--status", "all"]).await?;
        assert!(all.contains("4 report(s)"));

        let limited = harness
            .run(&["reports", "list", "--status", "all", "--limit", "1"])
            .await?;
        assert!(limited.contains("1 report(s)"));

        assert_usage(harness.run(&["reports", "list", "--status", "bogus"]).await)?;
        assert_usage(harness.run(&["reports", "list", "--limit", "0"]).await)?;
        assert_usage(harness.run(&["reports", "list", "--limit", "201"]).await)?;

        let json_output = harness
            .run(&["reports", "list", "--status", "all", "--json"])
            .await?;
        let items: Value = serde_json::from_str(&json_output)?;
        let items = items.as_array().ok_or("--json must print an array")?;
        assert_eq!(items.len(), 4);
        let first = &items[0];
        for key in [
            "id",
            "status",
            "reason",
            "created_at",
            "handled_at",
            "reporter_id",
            "group_id",
            "target_type",
            "message_id",
            "user_id",
            "snapshot",
        ] {
            assert!(first.get(key).is_some(), "json item lacks {key}: {first}");
        }
        let serialized = serde_json::to_string(&items)?;
        assert!(!serialized.contains("nickname"));
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn an_empty_queue_prints_no_reports_and_empty_json() -> TestResult {
    let harness = AdminHarness::new().await?;
    let result: TestResult = async {
        assert_eq!(harness.run(&["reports", "list"]).await?, "no reports\n");
        let json = harness.run(&["reports", "list", "--json"]).await?;
        assert_eq!(serde_json::from_str::<Value>(&json)?, serde_json::json!([]));
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn show_prints_the_stored_snapshot_and_escapes_terminal_control_characters() -> TestResult {
    let harness = AdminHarness::new().await?;
    let result: TestResult = async {
        let s = scenario(&harness).await?;
        let hostile = "red \u{1b}[31mansi\u{1b}[0m and \u{202e}reversed";
        let report = ReportFixture::message(s.reporter, s.group, s.message, hostile)
            .insert(&harness.pool)
            .await?;

        let shown = harness
            .run(&["reports", "show", &report.to_string()])
            .await?;
        assert!(shown.contains(&format!("report: {report}")));
        assert!(shown.contains("status: open"));
        assert!(shown.contains("reason: harassment"));
        assert!(shown.contains("snapshot content_type: text"));
        assert!(shown.contains("snapshot text: \"red "));
        assert!(
            !shown.contains('\u{1b}') && !shown.contains('\u{202e}'),
            "terminal control characters reached the output: {shown:?}"
        );
        assert!(shown.contains("\\u{001b}") && shown.contains("\\u{202e}"));

        let json = harness
            .run(&["reports", "show", &report.to_string(), "--json"])
            .await?;
        let item: Value = serde_json::from_str(&json)?;
        assert_eq!(item["id"], report.to_string());
        assert_eq!(item["snapshot"]["text"], hostile);
        assert_eq!(item["target_type"], "message");

        let user_report = ReportFixture::user(s.reporter, s.group, s.author)
            .insert(&harness.pool)
            .await?;
        let shown = harness
            .run(&["reports", "show", &user_report.to_string()])
            .await?;
        assert!(shown.contains(&format!("user {}", s.author)));
        assert!(!shown.contains("snapshot"));

        assert_failure(
            harness
                .run(&["reports", "show", &Uuid::new_v4().to_string()])
                .await,
            "not found",
        )?;
        assert_usage(harness.run(&["reports", "show", "not-a-uuid"]).await)?;
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn dismiss_and_resolve_set_the_status_idempotently_and_refuse_the_other_terminal_status()
-> TestResult {
    let harness = AdminHarness::new().await?;
    let result: TestResult = async {
        let s = scenario(&harness).await?;
        let to_dismiss = ReportFixture::user(s.reporter, s.group, s.author)
            .insert(&harness.pool)
            .await?;
        let to_resolve = ReportFixture::user(s.reporter, s.group, s.author)
            .insert(&harness.pool)
            .await?;

        let dismissed = harness
            .run(&["reports", "dismiss", &to_dismiss.to_string()])
            .await?;
        assert!(dismissed.contains("dismissed"));
        assert_eq!(
            report_status(&harness.pool, to_dismiss).await?,
            ("dismissed".to_owned(), true)
        );
        let again = harness
            .run(&["reports", "dismiss", &to_dismiss.to_string()])
            .await?;
        assert!(again.contains("already dismissed"));
        assert_failure(
            harness
                .run(&["reports", "resolve", &to_dismiss.to_string()])
                .await,
            "already dismissed",
        )?;
        assert_eq!(
            report_status(&harness.pool, to_dismiss).await?.0,
            "dismissed"
        );

        let resolved = harness
            .run(&["reports", "resolve", &to_resolve.to_string()])
            .await?;
        assert!(resolved.contains("resolved"));
        assert_eq!(
            report_status(&harness.pool, to_resolve).await?,
            ("actioned".to_owned(), true)
        );
        let again = harness
            .run(&["reports", "resolve", &to_resolve.to_string()])
            .await?;
        assert!(again.contains("already resolved"));
        assert_failure(
            harness
                .run(&["reports", "dismiss", &to_resolve.to_string()])
                .await,
            "already actioned",
        )?;

        assert_failure(
            harness
                .run(&["reports", "dismiss", &Uuid::new_v4().to_string()])
                .await,
            "not found",
        )?;
        assert_failure(
            harness
                .run(&["reports", "resolve", &Uuid::new_v4().to_string()])
                .await,
            "not found",
        )?;
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn purge_deletes_only_handled_reports_older_than_the_retention_with_their_alerts()
-> TestResult {
    let harness = AdminHarness::new().await?;
    let result: TestResult = async {
        let s = scenario(&harness).await?;
        let operator = insert_user(&harness.pool, "operator").await?;
        let installation = insert_installation(&harness.pool, operator).await?;
        let old_actioned = ReportFixture::message(s.reporter, s.group, s.message, "old")
            .handled("actioned", 400)
            .insert(&harness.pool)
            .await?;
        let old_dismissed = ReportFixture::user(s.reporter, s.group, s.author)
            .handled("dismissed", 366)
            .insert(&harness.pool)
            .await?;
        let recent_dismissed = ReportFixture::user(s.reporter, s.group, s.author)
            .handled("dismissed", 10)
            .insert(&harness.pool)
            .await?;
        let just_inside = ReportFixture::user(s.reporter, s.group, s.author)
            .handled("actioned", 364)
            .insert(&harness.pool)
            .await?;
        // Old but still open: an unhandled report is never purged.
        let old_open = ReportFixture::user(s.reporter, s.group, s.author)
            .created_days_ago(500)
            .insert(&harness.pool)
            .await?;
        // The operator alert occurrence of an old report must go with it (no ON DELETE).
        for report in [old_actioned, recent_dismissed] {
            sqlx::query(
                "INSERT INTO push_delivery_intents \
                     (id, report_id, recipient_user_id, push_installation_id, \
                      installation_owner_epoch, message_preview_enabled_snapshot, payload) \
                 SELECT gen_random_uuid(), $1, $2, id, owner_epoch, false, \
                        jsonb_build_object('type', 'report', 'report_id', $1::UUID::TEXT) \
                 FROM push_installations WHERE id = $3",
            )
            .bind(report)
            .bind(operator)
            .bind(installation)
            .execute(&harness.pool)
            .await?;
        }

        let output = harness.run(&["reports", "purge"]).await?;
        assert!(output.contains("purged 2 handled report(s)"), "{output}");
        assert!(
            output.contains("365 day(s)"),
            "default retention is one year: {output}"
        );
        for gone in [old_actioned, old_dismissed] {
            assert!(!report_exists(&harness.pool, gone).await?);
        }
        for kept in [recent_dismissed, just_inside, old_open] {
            assert!(report_exists(&harness.pool, kept).await?);
        }
        assert_eq!(
            count(
                &harness.pool,
                "SELECT count(*) FROM push_delivery_intents WHERE report_id = $1",
                old_actioned
            )
            .await?,
            0
        );
        assert_eq!(
            count(
                &harness.pool,
                "SELECT count(*) FROM push_delivery_intents WHERE report_id = $1",
                recent_dismissed
            )
            .await?,
            1,
            "the alert of a kept report stays"
        );

        let again = harness.run(&["reports", "purge"]).await?;
        assert!(again.contains("purged 0 handled report(s)"));

        // A shorter retention reaches the two handled reports left (10 and 364 days old); the
        // open report still stays.
        let shorter = harness
            .run(&["reports", "purge", "--older-than-days", "5"])
            .await?;
        assert!(shorter.contains("purged 2 handled report(s)"), "{shorter}");
        assert!(shorter.contains("5 day(s)"));
        assert!(!report_exists(&harness.pool, recent_dismissed).await?);
        assert!(!report_exists(&harness.pool, just_inside).await?);
        assert!(report_exists(&harness.pool, old_open).await?);

        assert_usage(
            harness
                .run(&["reports", "purge", "--older-than-days", "0"])
                .await,
        )?;
        assert_usage(
            harness
                .run(&["reports", "purge", "--older-than-days", "3651"])
                .await,
        )?;
        assert_usage(
            harness
                .run(&["reports", "purge", "--older-than-days", "x"])
                .await,
        )?;
        assert_usage(harness.run(&["reports", "purge", "extra"]).await)?;
        assert!(report_exists(&harness.pool, old_open).await?);
        Ok(())
    }
    .await;
    harness.finish(result).await
}
