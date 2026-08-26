use std::{fs, io};

use serde_json::Value;

use crate::TestResult;

#[test]
fn task_7_contract_contribution_is_the_exact_selected_topic_wire() -> TestResult {
    let operations: Value = serde_json::from_str(&fs::read_to_string(
        "contracts/contributions/task-7/dto/operations.json",
    )?)?;
    let rows = operations["operations"]
        .as_array()
        .ok_or_else(|| io::Error::other("task-7 operations must be an array"))?;
    assert_eq!(rows.len(), 7);
    for (index, expected) in [
        ("T1", "POST", "/api/v1/groups/{group_id}/topics"),
        ("T2", "GET", "/api/v1/groups/{group_id}/topics/dates"),
        ("T3", "GET", "/api/v1/groups/{group_id}/topics"),
        ("T4", "GET", "/api/v1/groups/{group_id}/topics/{topic_id}"),
        ("T5", "PATCH", "/api/v1/groups/{group_id}/topics/{topic_id}"),
        ("T6", "PUT", "/api/v1/groups/{group_id}/topics/{topic_id}/tags"),
        ("T7", "GET", "/api/v1/groups/{group_id}/topics/{topic_id}/tags"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(rows[index]["id"], expected.0);
        assert_eq!(rows[index]["method"], expected.1);
        assert_eq!(rows[index]["path"], expected.2);
    }
    assert_eq!(rows[0]["auth"], "member");
    assert_eq!(rows[0]["success_status"], "201 new|200 retry");
    assert_eq!(rows[4]["auth"], "author");
    assert_eq!(rows[5]["auth"], "author|owner");

    let schema: Value = serde_json::from_str(&fs::read_to_string(
        "contracts/contributions/task-7/schemas/topics-wire.schema.json",
    )?)?;
    assert_eq!(schema["$defs"]["TopicCreatedEvent"]["properties"]["type"]["const"], "topic.created");
    assert_eq!(schema["$defs"]["TopicCreatedEvent"]["properties"]["version"]["const"], 1);
    assert_eq!(schema["$defs"]["CanonicalTopic"]["properties"]["status"]["enum"], serde_json::json!(["seed", "enriched"]));
    assert_eq!(schema["$defs"]["TopicPatch"]["properties"]["title"]["type"], serde_json::json!(["string", "null"]));
    assert_eq!(
        schema["$defs"]["TagReplace"]["properties"]["tags"]["items"]["required"],
        serde_json::json!(["tag", "source"])
    );

    let fixture: Value = serde_json::from_str(&fs::read_to_string(
        "contracts/contributions/task-7/fixtures/topic-flow.json",
    )?)?;
    assert_eq!(fixture["t1"]["idempotency"]["same_payload"], "200 canonical existing");
    assert_eq!(fixture["t1"]["topic_created"]["is_distinct_from_announcement_event"], true);
    assert_eq!(fixture["t5"]["owner_non_author"], "403 topic_author_required");
    assert_eq!(fixture["transaction"]["rows"].as_array().map(Vec::len), Some(8));
    Ok(())
}
