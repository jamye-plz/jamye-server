use std::{fs, io};

use serde_json::Value;

use crate::TestResult;

#[test]
fn task_6_contract_contribution_is_the_exact_selected_group_wire() -> TestResult {
    let operations: Value = serde_json::from_str(&fs::read_to_string(
        "contracts/contributions/task-6/dto/operations.json",
    )?)?;
    let operations = operations["operations"]
        .as_array()
        .ok_or_else(|| io::Error::other("task-6 operations must be an array"))?;
    let actual = operations
        .iter()
        .map(|operation| {
            (
                operation["id"].as_str(),
                operation["method"].as_str(),
                operation["path"].as_str(),
                operation["success_status"].as_u64(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        vec![
            (Some("G1"), Some("POST"), Some("/api/v1/groups"), Some(201)),
            (Some("G2"), Some("GET"), Some("/api/v1/groups"), Some(200)),
            (
                Some("G3"),
                Some("GET"),
                Some("/api/v1/groups/{group_id}"),
                Some(200)
            ),
            (
                Some("G4"),
                Some("GET"),
                Some("/api/v1/groups/{group_id}/members"),
                Some(200)
            ),
            (
                Some("G5"),
                Some("PATCH"),
                Some("/api/v1/groups/{group_id}"),
                Some(200)
            ),
            (
                Some("G6"),
                Some("DELETE"),
                Some("/api/v1/groups/{group_id}"),
                Some(204)
            ),
            (
                Some("G7"),
                Some("DELETE"),
                Some("/api/v1/groups/{group_id}/members/{user_id}"),
                Some(204)
            ),
            (
                Some("G8"),
                Some("PATCH"),
                Some("/api/v1/groups/{group_id}/members/{user_id}"),
                Some(204)
            ),
            (
                Some("I1"),
                Some("POST"),
                Some("/api/v1/groups/{group_id}/invites"),
                Some(201)
            ),
            (
                Some("I2"),
                Some("POST"),
                Some("/api/v1/invites/{code}/join"),
                Some(200)
            ),
        ]
    );

    let schema: Value = serde_json::from_str(&fs::read_to_string(
        "contracts/contributions/task-6/schemas/groups-wire.schema.json",
    )?)?;
    let group = &schema["$defs"]["Group"];
    assert!(
        group["required"].as_array().is_some_and(
            |required| required.contains(&Value::String("main_chatroom_id".to_owned()))
        )
    );
    assert_eq!(group["properties"]["main_chatroom_id"]["type"], "string");
    let member = &schema["$defs"]["Member"];
    assert!(member["properties"].get("membership_id").is_none());

    let fixture: Value = serde_json::from_str(&fs::read_to_string(
        "contracts/contributions/task-6/fixtures/group-invite-flow.json",
    )?)?;
    assert_eq!(fixture["default_member_cap"], 12);
    assert_eq!(fixture["i2"]["existing_member"]["joined"], false);
    assert_eq!(fixture["i2"]["existing_member"]["use_delta"], 0);
    assert_eq!(fixture["realtime_revocation_owner"], "task-6c");
    Ok(())
}
