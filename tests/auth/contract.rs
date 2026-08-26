use std::io;

use serde_json::{Value, json};

use crate::TestResult;

#[test]
fn feature_contract_contribution_is_the_exact_selected_auth_wire() -> TestResult {
    let operations: Value = serde_json::from_str(include_str!(
        "../../contracts/contributions/task-5/dto/operations.json"
    ))?;
    assert_eq!(operations["contract_version"], "1");
    assert_eq!(
        operations["provider_path_allowlist"],
        json!(["kakao", "google"])
    );
    assert_eq!(operations["provider_path_deferred"], json!(["apple"]));
    let operation_rows = operations["operations"]
        .as_array()
        .ok_or_else(|| io::Error::other("task-5 operation contribution must be an array"))?;
    let operation_ids = operation_rows
        .iter()
        .map(|row| row["id"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        operation_ids,
        vec![
            Some("A1"),
            Some("A2"),
            Some("A3"),
            Some("A4"),
            Some("U1"),
            Some("U2")
        ]
    );
    assert_eq!(
        operation_rows[0]["path"],
        "/api/v1/auth/oauth/{provider}/authorize"
    );
    assert_eq!(operation_rows[2]["request"], "RefreshIn");
    assert_eq!(operation_rows[2]["response"], "TokenPair");

    let schema: Value = serde_json::from_str(include_str!(
        "../../contracts/contributions/task-5/schemas/auth-wire.schema.json"
    ))?;
    assert_eq!(
        schema["$defs"]["OAuthAuthorizeIn"]["properties"]["code_challenge_method"]["const"],
        "S256"
    );
    assert_eq!(
        schema["$defs"]["OAuthAuthorizeOut"]["properties"]["expires_in_seconds"]["const"],
        600
    );
    assert_eq!(
        schema["$defs"]["RefreshIn"]["required"],
        json!(["refresh_token"])
    );
    assert_eq!(
        schema["$defs"]["TokenPair"]["required"],
        json!([
            "token_type",
            "access_token",
            "access_token_expires_at",
            "refresh_token",
            "refresh_token_expires_at"
        ])
    );
    assert_eq!(
        schema["$defs"]["TokenPair"]["properties"]["token_type"]["const"],
        "Bearer"
    );
    Ok(())
}

#[test]
fn mobile_handoff_is_exactly_two_sends_and_one_refresh() -> TestResult {
    let handoff: Value = serde_json::from_str(include_str!(
        "../../contracts/contributions/task-5/fixtures/mobile-auth-handoff.json"
    ))?;
    assert_eq!(handoff["fixture"], "mobile_auth_two_send_one_refresh");
    assert_eq!(handoff["execution_owner"], "jamye-app");
    assert_eq!(handoff["server_execution"], "none");
    assert_eq!(handoff["single_flight"]["a3_network_request_count"], 1);
    assert_eq!(
        handoff["single_flight"]["waiters"],
        json!(["send_1", "send_2"])
    );
    assert_eq!(
        handoff["single_flight"]["success_order"],
        json!([
            "replace_secure_store_refresh_credential",
            "replay_send_1_once_with_original_client_msg_id",
            "replay_send_2_once_with_original_client_msg_id"
        ])
    );
    assert_eq!(
        handoff["retry_taxonomy"]["reauthenticate_without_loop"],
        json!([
            "refresh_token_invalid",
            "refresh_token_reused",
            "second_401_after_replay"
        ])
    );
    assert_eq!(handoff["task_12_role"], "audit_and_assemble_only");
    Ok(())
}
