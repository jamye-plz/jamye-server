use std::{fs, io};

use serde_json::Value;

use crate::TestResult;

#[test]
fn task_6b_contract_contribution_is_the_exact_selected_chatroom_wire() -> TestResult {
    let operations: Value = serde_json::from_str(&fs::read_to_string(
        "contracts/contributions/task-6b/dto/operations.json",
    )?)?;
    let operations = operations["operations"]
        .as_array()
        .ok_or_else(|| io::Error::other("task-6b operations must be an array"))?;
    let actual = operations
        .iter()
        .map(|operation| {
            (
                operation["id"].as_str(),
                operation["method"].as_str(),
                operation["path"].as_str(),
                operation["success_status"].as_u64(),
                operation["pagination"].as_str(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        vec![
            (
                Some("C1"),
                Some("GET"),
                Some("/api/v1/groups/{group_id}/chatrooms"),
                Some(200),
                Some("after+limit")
            ),
            (
                Some("C2"),
                Some("GET"),
                Some("/api/v1/chatrooms/{chatroom_id}/messages"),
                Some(200),
                Some("before+limit")
            ),
            (
                Some("C3"),
                Some("POST"),
                Some("/api/v1/chatrooms/{chatroom_id}/read"),
                Some(200),
                None
            ),
        ]
    );

    let schema: Value = serde_json::from_str(&fs::read_to_string(
        "contracts/contributions/task-6b/schemas/chatrooms-wire.schema.json",
    )?)?;
    let message = &schema["$defs"]["DenormalizedMessage"];
    for field in ["sender_nickname", "sender_avatar_url", "media"] {
        assert!(message["properties"].get(field).is_some());
    }
    assert_eq!(message["properties"]["media"]["maxItems"], 0);
    assert_eq!(
        schema["$defs"]["ReadCursorIn"]["properties"]["cursor"]["type"],
        "string"
    );
    assert!(
        schema["$defs"]["ReadMarker"]["properties"]
            .get("last_read_at")
            .is_none()
    );

    let fixture: Value = serde_json::from_str(&fs::read_to_string(
        "contracts/contributions/task-6b/fixtures/chatroom-history-read.json",
    )?)?;
    assert_eq!(
        fixture["c2"]["page"]["items"][0]["media"],
        serde_json::json!([])
    );
    assert_eq!(
        fixture["c2"]["page"]["items"][1]["sender_nickname"],
        Value::Null
    );
    assert_eq!(fixture["c3"]["canonical_last_read_cursor"], "140");
    assert_eq!(fixture["c3"]["response"]["last_read_cursor"], "140");
    assert_eq!(fixture["c3"]["row_count"], 1);
    assert_eq!(fixture["c3"]["unknown_cursor_mutates"], false);
    assert_eq!(
        fixture["future_composition"]["mark_conversation_read_owner"],
        "task-12"
    );
    Ok(())
}
