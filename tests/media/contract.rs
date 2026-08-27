use std::{fs, io};

use serde_json::Value;

use crate::TestResult;

const CONTRIBUTION_ROOT: &str = "contracts/contributions/task-8";

#[test]
fn md1_through_md5_contract_contribution_matches_the_selected_inventory() -> TestResult {
    let operations = read_json(&format!("{CONTRIBUTION_ROOT}/dto/operations.json"))?;
    let rows = operations["operations"]
        .as_array()
        .ok_or_else(|| io::Error::other("task-8 operations must be an array"))?;
    let actual = rows
        .iter()
        .map(|row| {
            (
                row["id"].as_str(),
                row["method"].as_str(),
                row["path"].as_str(),
                row["success_status"].as_u64(),
                row["pagination"].as_str(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        vec![
            (
                Some("MD1"),
                Some("POST"),
                Some("/api/v1/media/uploads"),
                Some(201),
                None,
            ),
            (
                Some("MD2"),
                Some("POST"),
                Some("/api/v1/media/uploads/{upload_id}/finalize"),
                Some(200),
                None,
            ),
            (
                Some("MD3"),
                Some("GET"),
                Some("/api/v1/topics/{topic_id}/media"),
                Some(200),
                Some("after+limit"),
            ),
            (
                Some("MD4"),
                Some("GET"),
                Some("/api/v1/media/{media_id}/url"),
                Some(200),
                None,
            ),
            (
                Some("MD5"),
                Some("GET"),
                Some("/api/v1/media/{media_id}/download"),
                Some(307),
                None,
            ),
        ]
    );
    assert_eq!(rows[0]["response"], "UploadIntent+PresignedPut");
    assert_eq!(rows[1]["response"], "UploadFinalizeResult");
    assert_eq!(rows[2]["response"], "TopicMediaPage");

    let schema = read_json(&format!(
        "{CONTRIBUTION_ROOT}/schemas/media-wire.schema.json"
    ))?;
    let definitions = schema["$defs"]
        .as_object()
        .ok_or_else(|| io::Error::other("task-8 media schema definitions are missing"))?;
    for name in [
        "UploadIntentCreate",
        "UploadIntent",
        "PresignedPut",
        "UploadFinalize",
        "ConfirmedUpload",
        "ChatUploadFinalizeResult",
        "TopicUploadFinalizeResult",
        "UploadFinalizeResult",
        "TopicMedia",
        "TopicMediaPage",
        "MediaAccessUrl",
        "MessageAttachment",
    ] {
        assert!(
            definitions.contains_key(name),
            "missing task-8 schema: {name}"
        );
    }
    assert_eq!(
        definitions["UploadFinalizeResult"]["discriminator"]["propertyName"],
        "scope"
    );
    assert_eq!(
        definitions["UploadFinalizeResult"]["oneOf"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        definitions["MessageAttachment"]["required"],
        serde_json::json!([
            "id",
            "media_upload_id",
            "type",
            "byte_size",
            "width",
            "height",
            "duration",
            "filename",
            "position"
        ])
    );
    assert!(
        definitions["MessageAttachment"]["properties"]
            .get("object_key")
            .is_none()
    );
    assert!(
        definitions["MediaAccessUrl"]["properties"]
            .get("object_key")
            .is_none()
    );
    Ok(())
}

#[test]
fn history_and_topic_contracts_expose_canonical_media_without_private_chat_keys() -> TestResult {
    let history_schema =
        read_json("contracts/contributions/task-6b/schemas/chatrooms-wire.schema.json")?;
    let history_media = &history_schema["$defs"]["DenormalizedMessage"]["properties"]["media"];
    assert_eq!(history_media["maxItems"], 4);
    assert_eq!(
        history_media["items"]["$ref"],
        "https://contracts.jamye.test/task-8/media-wire.schema.json#/$defs/MessageAttachment"
    );

    let topic_schema = read_json("contracts/contributions/task-7/schemas/topics-wire.schema.json")?;
    let topic_media = &topic_schema["$defs"]["TopicMedia"];
    assert_eq!(
        topic_media["properties"]["media_upload_id"]["format"],
        "uuid"
    );
    assert!(
        topic_media["required"]
            .as_array()
            .is_some_and(|required| required.iter().any(|field| field == "media_upload_id"))
    );

    let history_fixture =
        read_json("contracts/contributions/task-6b/fixtures/chatroom-history-read.json")?;
    let media = history_fixture["c2"]["page"]["items"][0]["media"]
        .as_array()
        .ok_or_else(|| io::Error::other("history fixture media must be an array"))?;
    assert_eq!(media.len(), 2);
    assert_eq!(media[0]["position"], 0);
    assert_eq!(media[1]["position"], 1);
    assert!(media.iter().all(|item| item.get("object_key").is_none()));
    Ok(())
}

#[test]
fn voice_fixture_covers_finalize_send_realtime_delta_history_and_reissue() -> TestResult {
    let fixture = read_json(&format!("{CONTRIBUTION_ROOT}/fixtures/media-flow.json"))?;
    let voice = &fixture["voice"];
    let attachment = &voice["attachment"];

    assert_eq!(voice["md1"]["request"]["scope"], "chat");
    assert_eq!(voice["md2"]["response"]["scope"], "chat");
    assert_eq!(voice["md2"]["response"]["status"], "confirmed");
    assert_eq!(voice["md2"]["response"]["bound"], false);
    assert!(voice["c4"]["request"]["body"].is_null());
    assert_eq!(
        voice["c4"]["request"]["media"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        voice["c4"]["request"]["client_msg_id"],
        voice["c4"]["retry"]["client_msg_id"]
    );
    assert_eq!(attachment["type"], "audio/ogg");
    assert!(attachment["duration"].as_u64().is_some());
    assert!(attachment.get("object_key").is_none());

    for projection in ["message_created", "delta", "history"] {
        assert_eq!(
            &voice["delivery"][projection]["data"]["media"][0], attachment,
            "voice attachment drifted in {projection}"
        );
    }
    assert_eq!(
        voice["reissue"]["md4"]["request"]["media_id"],
        attachment["id"]
    );
    assert_eq!(
        voice["reissue"]["md5"]["request"]["media_id"],
        attachment["id"]
    );
    assert!(
        voice["reissue"]["md4"]["request"]
            .get("object_key")
            .is_none()
    );
    assert!(
        voice["reissue"]["md5"]["request"]
            .get("object_key")
            .is_none()
    );
    Ok(())
}

fn read_json(path: &str) -> TestResult<Value> {
    let source = fs::read_to_string(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            io::Error::other(format!(
                "RED: {path} is absent; task-8 must publish its exact media DTO/schema/fixture contribution"
            ))
        } else {
            error
        }
    })?;
    Ok(serde_json::from_str(&source)?)
}
