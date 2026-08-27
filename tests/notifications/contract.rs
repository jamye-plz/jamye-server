use std::{fs, io};

use serde_json::Value;

use crate::TestResult;

const CONTRIBUTION_ROOT: &str = "contracts/contributions/task-9";

#[test]
fn n1_n2_and_p2_through_p4_contracts_freeze_d9_a_without_private_state() -> TestResult {
    let operations = read_json(&format!("{CONTRIBUTION_ROOT}/dto/operations.json"))?;
    let rows = operations["operations"]
        .as_array()
        .ok_or_else(|| io::Error::other("task-9 operations must be an array"))?;
    let actual = rows
        .iter()
        .map(|row| {
            (
                row["id"].as_str(),
                row["method"].as_str(),
                row["path"].as_str(),
                row["success_status"].clone(),
                row["pagination"].as_str(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        vec![
            (
                Some("N1"),
                Some("GET"),
                Some("/api/v1/notifications"),
                serde_json::json!(200),
                Some("after+limit"),
            ),
            (
                Some("N2"),
                Some("POST"),
                Some("/api/v1/notifications/{notification_id}/read"),
                serde_json::json!(204),
                None,
            ),
            (
                Some("P2"),
                Some("POST"),
                Some("/api/v1/push/installations"),
                serde_json::json!("201 create|200 canonical upsert"),
                None,
            ),
            (
                Some("P3"),
                Some("PUT"),
                Some("/api/v1/push/installations/{installation_id}"),
                serde_json::json!(200),
                None,
            ),
            (
                Some("P4"),
                Some("DELETE"),
                Some("/api/v1/push/installations/{installation_id}"),
                serde_json::json!(204),
                None,
            ),
        ]
    );

    let schema = read_json(&format!(
        "{CONTRIBUTION_ROOT}/schemas/notifications-push-wire.schema.json"
    ))?;
    let definitions = schema["$defs"]
        .as_object()
        .ok_or_else(|| io::Error::other("task-9 schema definitions are missing"))?;
    for name in [
        "Notification",
        "NotificationArgs",
        "NotificationPage",
        "ExpoInstallationCreate",
        "ExpoInstallationPut",
        "PushInstallation",
        "PushTapHandoff",
    ] {
        assert!(
            definitions.contains_key(name),
            "missing task-9 schema: {name}"
        );
    }

    let notification = &definitions["Notification"];
    assert_eq!(
        notification["required"],
        serde_json::json!([
            "id",
            "type",
            "args",
            "topic_id",
            "conversation_id",
            "source_cursor",
            "read_at",
            "created_at"
        ])
    );
    assert_eq!(
        notification["properties"]["type"]["enum"],
        serde_json::json!(["new_topic", "chat_unread", "other"])
    );
    assert_eq!(
        notification["properties"]["args"]["$ref"],
        "#/$defs/NotificationArgs"
    );
    for private in [
        "user_id",
        "payload",
        "dedup_key",
        "title",
        "body",
        "message",
    ] {
        assert!(
            notification["properties"].get(private).is_none(),
            "notification wire exposes private/server-rendered field: {private}"
        );
    }
    assert_eq!(
        definitions["NotificationPage"]["required"],
        serde_json::json!(["items", "next_cursor", "unread_count"])
    );

    let installation = &definitions["PushInstallation"];
    for private in ["id", "user_id", "owner_epoch", "token", "expo_token"] {
        assert!(
            installation["properties"].get(private).is_none(),
            "push installation wire exposes private identity: {private}"
        );
    }
    assert_eq!(
        definitions["ExpoInstallationCreate"]["required"],
        serde_json::json!(["platform", "environment", "installation_id", "expo_token"])
    );
    assert_eq!(
        definitions["ExpoInstallationCreate"]["properties"]["message_preview_enabled"]["default"],
        false
    );
    assert_eq!(
        definitions["ExpoInstallationPut"]["required"],
        serde_json::json!(["expo_token"])
    );

    let handoff = &definitions["PushTapHandoff"];
    assert_eq!(
        handoff["required"],
        serde_json::json!(["type", "notification_id", "conversation_id", "message_id"])
    );
    for forbidden in ["args", "title", "body", "preview", "payload", "user_id"] {
        assert!(
            handoff["properties"].get(forbidden).is_none(),
            "push-tap handoff exposes source data: {forbidden}"
        );
    }
    Ok(())
}

#[test]
fn fixture_covers_structured_localization_occurrence_identity_and_delta_first_handoff() -> TestResult
{
    let fixture = read_json(&format!(
        "{CONTRIBUTION_ROOT}/fixtures/notifications-push-flow.json"
    ))?;
    let notification = &fixture["d9_a"]["notification"];
    assert_eq!(notification["type"], "chat_unread");
    assert!(notification["args"].is_object());
    for forbidden in ["user_id", "payload", "dedup_key", "title", "body"] {
        assert!(notification.get(forbidden).is_none());
    }

    assert_eq!(fixture["p2"]["omitted_message_preview_enabled"], false);
    assert_eq!(fixture["p2"]["provider"], "expo");
    assert_eq!(
        fixture["occurrences"]["same_coalesced_notification_id"],
        true
    );
    assert_eq!(fixture["occurrences"]["distinct_source_event_count"], 2);
    assert_eq!(
        fixture["occurrences"]["retry_creates_new_occurrence"],
        false
    );

    let handoff = fixture["push_tap"]["payload"]
        .as_object()
        .ok_or_else(|| io::Error::other("push-tap fixture payload must be an object"))?;
    let mut keys = handoff.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["conversation_id", "message_id", "notification_id", "type"]
    );
    assert_eq!(fixture["push_tap"]["client_handoff"], "delta_first_sync");
    assert_eq!(fixture["push_tap"]["server_executes_navigation"], false);

    for forbidden in [
        "raw_message_body",
        "rendered_preview",
        "authorization",
        "expo_token",
        "user_id",
    ] {
        assert!(
            !serde_json::to_string(&fixture)?.contains(forbidden),
            "task-9 fixture exposes forbidden material: {forbidden}"
        );
    }
    Ok(())
}

fn read_json(path: &str) -> TestResult<Value> {
    let source = fs::read_to_string(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            io::Error::other(format!(
                "RED: {path} is absent; task-9 must publish its exact D9=A notification and Expo push contribution"
            ))
        } else {
            error
        }
    })?;
    Ok(serde_json::from_str(&source)?)
}
