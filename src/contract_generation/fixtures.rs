//! Deterministic C0 contract fixtures.

use serde_json::{Value, json};

use super::selected;

pub const SQLITE_ATOMICITY_SENTENCE: &str = "One exclusive SQLite transaction creates both messages(status=pending) and the persisted outbox command with the unchanged client_msg_id; after process reopen, both rows are visible or neither is.";

pub fn documents() -> Vec<(String, Value)> {
    vec![
        (
            "fixtures/c4-content-validation.json".to_owned(),
            c4_content_validation(),
        ),
        ("fixtures/c4-duplicate.json".to_owned(), c4_duplicate()),
        ("fixtures/c4-normal.json".to_owned(), c4_normal()),
        ("fixtures/c4-permission.json".to_owned(), c4_permission()),
        (
            "fixtures/message.created.json".to_owned(),
            message_created(),
        ),
        (
            "fixtures/manifest-provenance.json".to_owned(),
            manifest_provenance(),
        ),
        (
            "fixtures/mobile-sync-handoff.json".to_owned(),
            mobile_sync_handoff(),
        ),
        (
            "fixtures/realtime-lifecycle.json".to_owned(),
            realtime_lifecycle(),
        ),
        (
            "fixtures/unknown-event-recovery.json".to_owned(),
            unknown_event_recovery(),
        ),
        (
            "fixtures/version-negotiation.json".to_owned(),
            version_negotiation(),
        ),
    ]
}

pub fn documents_release_candidate() -> Vec<(String, Value)> {
    let mut documents = documents();
    documents.extend([
        (
            "fixtures/c2-health-profile-account.json".to_owned(),
            json!({
                "fixture": "c2_health_profile_account_selected_surfaces",
                "operation_ids": ["H1", "H2", "U1", "U2", "U3"],
                "evidence": "existing_owner_behavior_tests_named_by_selected-surface-mapping",
                "task_12_role": "declarative_mapping_gap_closure"
            }),
        ),
        (
            "fixtures/selected-surface-mapping.json".to_owned(),
            selected_surface_mapping(),
        ),
        (
            "fixtures/c2-mobile-handoff.json".to_owned(),
            c2_mobile_handoff(),
        ),
        (
            "fixtures/c2-bodyless-audio-reachability.json".to_owned(),
            c2_bodyless_audio_reachability(),
        ),
    ]);
    documents
}

fn selected_surface_mapping() -> Value {
    let rest_operations = selected::REST_SURFACES
        .iter()
        .map(|surface| {
            json!({
                "operation_id": surface.operation_id,
                "method": surface.method,
                "path": surface.path,
                "handler": surface.handler,
                "handler_route_probe": selected::ROUTE_PROBE,
                "feature_behavior_test": surface.feature_behavior_test,
                "fixture": surface.fixture,
            })
        })
        .collect::<Vec<_>>();
    let realtime_events = selected::REALTIME_SURFACES
        .iter()
        .map(|surface| {
            json!({
                "event_type": surface.event_type,
                "version": surface.version,
                "handler": surface.handler,
                "handler_route_probe": selected::ROUTE_PROBE,
                "feature_behavior_test": surface.feature_behavior_test,
                "fixture": surface.fixture,
                "schema": surface.schema,
            })
        })
        .collect::<Vec<_>>();
    json!({"rest_operations": rest_operations, "realtime_events": realtime_events})
}

fn c2_mobile_handoff() -> Value {
    json!({
        "atomicity_contract": SQLITE_ATOMICITY_SENTENCE,
        "references": {
            "task_5_auth_trace": "contracts/contributions/task-5/fixtures/mobile-auth-handoff.json",
            "task_3b_task_4b_two_phase_delta": "contracts/fixtures/mobile-sync-handoff.json;tests/realtime/c1.rs::dev_c1_flows_from_seed_to_rest_outbox_redis_websocket_and_delta"
        },
        "execution_owner": "jamye-app",
        "server_execution": "none"
    })
}

fn c2_bodyless_audio_reachability() -> Value {
    json!({
        "c4": {
            "request": {
                "body": null,
                "media": [{"state": "finalized", "mime_type": "audio/m4a"}],
                "client_msg_id": "30000000-0000-4000-8000-000000000001"
            },
            "retry": {"client_msg_id": "30000000-0000-4000-8000-000000000001"}
        },
        "server_path": {
            "composition": "SendMessage",
            "delivery": [
                "worker", "Redis", "WebSocket message.created", "offline delta", "history",
                "MD4 metadata-only reissue", "MD5 metadata-only reissue"
            ]
        },
        "mobile_execution": {"playback": "jamye-app-owned", "server_execution": "none"}
    })
}

fn canonical_message() -> Value {
    json!({
        "id": "20000000-0000-4000-8000-000000000001",
        "chatroom_id": "10000000-0000-4000-8000-000000000001",
        "sender_id": "10000000-0000-4000-8000-000000000002",
        "client_msg_id": "30000000-0000-4000-8000-000000000001",
        "body": "안녕하세요",
        "type": "user",
        "created_at": "2026-08-22T00:00:00Z",
        "media": []
    })
}

fn error(code: &str, message: &str) -> Value {
    json!({
        "error": {
            "code": code,
            "message": message,
            "request_id": "40000000-0000-4000-8000-000000000001",
            "details": null
        }
    })
}

fn c4_normal() -> Value {
    json!({
        "fixture": "c4_normal",
        "operation_id": "C4",
        "request": {
            "method": "POST",
            "path": "/api/v1/chatrooms/10000000-0000-4000-8000-000000000001/messages",
            "headers": {"authorization": "Bearer <redacted>"},
            "body": {
                "client_msg_id": "30000000-0000-4000-8000-000000000001",
                "body": "안녕하세요",
                "media": []
            }
        },
        "response": {"status": 201, "body": canonical_message()},
        "database_effect": "one message, one conversation event, and one outbox intent commit atomically"
    })
}

fn c4_duplicate() -> Value {
    json!({
        "fixture": "c4_duplicate_D8_A",
        "operation_id": "C4",
        "first": {
            "request": {
                "headers": {},
                "body": {
                    "client_msg_id": "30000000-0000-4000-8000-000000000001",
                    "body": "안녕하세요",
                    "media": []
                }
            },
            "response": {"status": 201, "body": canonical_message()}
        },
        "same_payload_retry": {
            "request": {
                "headers": {"Idempotency-Key": "30000000-0000-4000-8000-000000000001"},
                "body": {
                    "client_msg_id": "30000000-0000-4000-8000-000000000001",
                    "body": "안녕하세요",
                    "media": []
                }
            },
            "response": {"status": 200, "body": canonical_message()},
            "additional_logical_messages": 0
        },
        "changed_payload_retry": {
            "request": {
                "headers": {},
                "body": {
                    "client_msg_id": "30000000-0000-4000-8000-000000000001",
                    "body": "변경된 본문",
                    "media": []
                }
            },
            "response": {
                "status": 409,
                "body": error("idempotency_conflict", "같은 메시지 키에 다른 내용이 사용되었습니다.")
            },
            "mutation_count": 0
        }
    })
}

fn c4_permission() -> Value {
    json!({
        "fixture": "c4_permission",
        "operation_id": "C4",
        "indistinguishable_cases": ["missing_conversation", "cross_group", "nonmember", "deleted_group"],
        "response": {
            "status": 403,
            "body": error("membership_required", "이 그룹에 접근할 수 없습니다.")
        },
        "mutation_count": 0
    })
}

fn c4_content_validation() -> Value {
    json!({
        "fixture": "c4_content_and_header_validation",
        "operation_id": "C4",
        "message_content_required": {
            "cases": [
                {"body_state": "missing", "media_state": "missing"},
                {"body_state": "null", "media_state": "empty"},
                {"body_state": "empty_string", "media_state": "missing"}
            ],
            "response": {
                "status": 422,
                "body": error("message_content_required", "메시지 본문 또는 미디어가 필요합니다.")
            },
            "mutation_count": 0
        },
        "optional_idempotency_header_mismatch": {
            "header": "30000000-0000-4000-8000-000000000099",
            "body_client_msg_id": "30000000-0000-4000-8000-000000000001",
            "response": {
                "status": 422,
                "body": error("idempotency_key_mismatch", "Idempotency-Key가 client_msg_id와 일치하지 않습니다.")
            },
            "mutation_count": 0
        },
        "whitespace_only_body": {
            "request_body": "   ",
            "accepted": true,
            "canonical_body": "   ",
            "server_trim": false
        }
    })
}

fn message_created() -> Value {
    json!({
        "fixture": "message.created",
        "event": {
            "version": 1,
            "type": "message.created",
            "event_id": "50000000-0000-4000-8000-000000000001",
            "conversation_id": "10000000-0000-4000-8000-000000000001",
            "cursor": "101",
            "occurred_at": "2026-08-22T00:00:00Z",
            "data": canonical_message()
        },
        "delivery_may_repeat": true,
        "consumer_idempotency_key": "event_id",
        "cursor_commits_only_after_application": true
    })
}

fn manifest_provenance() -> Value {
    json!({
        "fixture": "explicit_manifest_provenance",
        "pre_publication": {
            "input": {
                "server_tag": null,
                "server_commit": "dirty",
                "contract_version": "1",
                "server_version": "0.1.0"
            },
            "dirty_workspace_result": "uses_the_explicit_input",
            "clean_checkout_result": "still_uses_the_explicit_dirty_input",
            "git_head_inference": false
        },
        "future_separately_authorized_publication": {
            "step_1": "establish_the_source_commit",
            "step_2": "generate_an_artifact_commit_and_tag_whose_manifest_points_to_the_source_commit",
            "manifest_server_commit": "0123456789abcdef0123456789abcdef01234567",
            "manifest_server_tag": "v0.1.0",
            "artifact_commit_points_to_itself": false,
            "current_plan_executes_transition": false
        }
    })
}

fn version_negotiation() -> Value {
    json!({
        "fixture": "current_previous_version_negotiation",
        "header": "X-Jamye-Contract-Version",
        "accepted": [
            {"requested": "1", "echoed": "1"},
            {"requested": "0", "echoed": "0"}
        ],
        "unsupported": {
            "requested": "999",
            "response": {
                "status": 426,
                "body": error("contract_upgrade_required", "지원되는 계약 버전으로 앱을 업데이트해 주세요.")
            },
            "automatic_retry": false,
            "ticket_created": false
        }
    })
}

fn realtime_lifecycle() -> Value {
    json!({
        "fixture": "realtime_ticket_join_and_terminal_cleanup",
        "ticket": {
            "issued_once": true,
            "stored_form": "sha256_digest_only",
            "consume": "atomic_once",
            "contract_version": "1",
            "effective_expiry": "min(issue_time_plus_30_seconds, access_token_exp)"
        },
        "join": {
            "request": {
                "type": "subscribe",
                "request_id": "60000000-0000-4000-8000-000000000001",
                "conversation_id": "10000000-0000-4000-8000-000000000001"
            },
            "ack_after_authorization_and_registration": {
                "type": "subscribed",
                "request_id": "60000000-0000-4000-8000-000000000001",
                "conversation_id": "10000000-0000-4000-8000-000000000001"
            }
        },
        "denied_subscribe_after_zero_or_more_valid_subscriptions": {
            "data_frame_count": 0,
            "error_frame_count": 0,
            "registry_entries_after_cleanup": 0,
            "close": {"code": 4001, "reason": "membership_required"},
            "client_cleanup_class": "evicted"
        },
        "D13_A": {
            "logout_before_access_exp": "access and an otherwise valid unconsumed ticket remain valid until their existing effective expiry",
            "at_bound_access_exp": {
                "registry_entries_after_cleanup": 0,
                "close": {"code": 4401, "reason": "realtime_auth_expired"}
            },
            "next_action": "refresh_or_reauthenticate_then_new_ticket_then_delta_first"
        },
        "ticket_invalid_expired_reused_or_lost": {
            "close": {"code": 4401, "reason": "realtime_auth_failed"}
        }
    })
}

fn mobile_sync_handoff() -> Value {
    json!({
        "fixture": "declarative_mobile_sqlite_outbox_and_two_phase_sync",
        "execution_owner": "jamye-app",
        "server_execution": "none",
        "atomicity_contract": SQLITE_ATOMICITY_SENTENCE,
        "outbox": {
            "process_reopen": "reload the same persisted command",
            "retry_client_msg_id": "unchanged",
            "network_regain": "reconcile to exactly one canonical server message",
            "retryable": ["network", "timeout", "429", "5xx"],
            "terminal_or_user_action": ["403", "404", "409_idempotency_conflict", "422"],
            "upgrade_stop": "426_contract_upgrade_required",
            "ordinary_401_marker": {
                "preserve_outbox_intent": true,
                "preserve_client_msg_id": true,
                "next_contract_owner": "task-5"
            }
        },
        "triggers": ["app_foreground", "network_regain", "websocket_reconnect", "validated_push_tap"],
        "connect_sequence": [
            "persist_last_applied_cursor",
            "drain_S1_phase_1_to_exhaustion",
            "issue_one_time_R1_ticket",
            "connect_and_receive_subscribed_ack",
            "drain_S1_phase_2_to_exhaustion"
        ],
        "page_fixture": {
            "limit": 2,
            "phase_1_pages": [
                {"items": ["event_1", "event_2"], "next_cursor": "2"},
                {"items": ["event_3", "event_4"], "next_cursor": "4"},
                {"items": ["event_5"], "next_cursor": "5"},
                {"items": [], "next_cursor": null}
            ],
            "between_phase_1_and_ack": "event_A",
            "between_ack_and_phase_2": "event_B",
            "websocket_delivery": ["event_A", "event_B", "event_B"],
            "phase_2_pages": [
                {"items": ["event_A", "event_B"], "next_cursor": "7"},
                {"items": [], "next_cursor": null}
            ],
            "same_or_regressing_next_cursor": "stop_with_safe_diagnostics_and_resume_from_last_committed_cursor",
            "bounded_page_guard": "persist_committed_progress_then_stop"
        },
        "app_owned_invariants": {
            "pending_and_outbox": "both_or_neither_after_reopen",
            "applied_events": "UNIQUE(event_id)",
            "cursor": "per_conversation_monotonic_compare_and_set_after_event_transaction_commit",
            "apply_path": "one applyEvent(event_id) path for both REST phases and WebSocket"
        },
        "server_observer": {"seen_event_ids": [], "last_cursor": null},
        "expected": {"event_loss": 0, "logical_applications_per_event": 1, "final_cursor": "7"}
    })
}

fn unknown_event_recovery() -> Value {
    json!({
        "fixture": "unknown_ws_to_bounded_delta_marker_then_known_and_reconcile",
        "unknown_websocket_input": {
            "type": "future.event",
            "event_id": "70000000-0000-4000-8000-000000000001",
            "cursor": "201"
        },
        "after_unknown_ws": {
            "cursor": "200",
            "action": "request_S1_after_200",
            "diagnostic_contains_payload": false
        },
        "S1_page": {
            "items": [
                {
                    "event_id": "70000000-0000-4000-8000-000000000001",
                    "cursor": "201",
                    "reconcile_scope": "chat_history"
                },
                {
                    "version": 1,
                    "type": "message.created",
                    "event_id": "70000000-0000-4000-8000-000000000002",
                    "conversation_id": "10000000-0000-4000-8000-000000000001",
                    "cursor": "202",
                    "occurred_at": "2026-08-22T00:00:01Z",
                    "data": canonical_message()
                }
            ],
            "next_cursor": null
        },
        "declarative_app_commit_order": [
            "insert_applied_events_U_as_unsupported_mark_chat_history_dirty_and_CAS_cursor_201",
            "apply_known_K_and_CAS_cursor_202",
            "refresh_C2_chat_history",
            "clear_dirty_state"
        ],
        "next_S1_page": {"items": [], "next_cursor": null},
        "expected": {
            "final_cursor": "202",
            "crash": false,
            "cursor_regression": false,
            "retry_storm": false,
            "realtime_variant_count": 1
        }
    })
}
