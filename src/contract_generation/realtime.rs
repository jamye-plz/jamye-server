//! C0 realtime JSON Schema and protocol documents.

use std::collections::BTreeSet;

use schemars::{JsonSchema, generate::SchemaSettings};
use serde_json::Value;

use super::model::{ClientFrame, MessageCreatedEvent, ServerFrame};
use super::{BoxError, invalid_data};

pub const REALTIME_DISCRIMINANTS: &[&str] = &["message.created"];

pub fn documents() -> Result<Vec<(String, Value)>, BoxError> {
    validate_discriminants()?;
    Ok(vec![
        (
            "realtime/client-frame.schema.json".to_owned(),
            schema_document::<ClientFrame>(
                "https://contracts.jamye.local/realtime/client-frame.schema.json",
                "Jamye realtime client frames",
            )?,
        ),
        (
            "realtime/message.created.schema.json".to_owned(),
            schema_document::<MessageCreatedEvent>(
                "https://contracts.jamye.local/realtime/message.created.schema.json",
                "Jamye message.created event v1",
            )?,
        ),
        (
            "realtime/protocol.json".to_owned(),
            protocol_document(),
        ),
        (
            "realtime/server-frame.schema.json".to_owned(),
            schema_document::<ServerFrame>(
                "https://contracts.jamye.local/realtime/server-frame.schema.json",
                "Jamye realtime server frames",
            )?,
        ),
    ])
}

fn schema_document<T: JsonSchema>(schema_id: &str, title: &str) -> Result<Value, BoxError> {
    let schema = SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<T>();
    let mut value = serde_json::to_value(schema)?;
    let root = value
        .as_object_mut()
        .ok_or_else(|| invalid_data("JSON Schema root must be an object"))?;
    root.insert("$id".to_owned(), Value::String(schema_id.to_owned()));
    root.insert("title".to_owned(), Value::String(title.to_owned()));
    Ok(value)
}

fn validate_discriminants() -> Result<(), BoxError> {
    if REALTIME_DISCRIMINANTS != ["message.created"] {
        return Err(invalid_data("C0 must contain only message.created").into());
    }
    let unique = REALTIME_DISCRIMINANTS.iter().copied().collect::<BTreeSet<_>>();
    if unique.len() != REALTIME_DISCRIMINANTS.len() {
        return Err(invalid_data("duplicate C0 realtime discriminant").into());
    }
    Ok(())
}

fn protocol_document() -> Value {
    serde_json::json!({
        "stage": "C0",
        "url": "wss://{host}/api/v1/realtime/ws?ticket={one_time_ticket}",
        "contract_versions": {
            "current": "1",
            "previous": "0",
            "request_header": "X-Jamye-Contract-Version",
            "unsupported": {
                "status": 426,
                "error_code": "contract_upgrade_required",
                "automatic_retry": false
            }
        },
        "ticket": {
            "storage": "redis_ephemeral_only",
            "raw_value": "csprng_256_bit_returned_once",
            "stored_key": "sha256_of_raw_ticket",
            "stored_value": [
                "user_id",
                "verified_access_token_sid",
                "contract_version",
                "access_token_expires_at"
            ],
            "consume": "atomic_getdel_or_equivalent_lua",
            "effective_expiry": "min(issue_time_plus_30_seconds, access_token_exp)",
            "invalid_expired_reused_or_redis_restart": {
                "close_code": 4401,
                "reason": "realtime_auth_failed"
            }
        },
        "client_frames": ["subscribe", "unsubscribe", "ping"],
        "server_control_frames": ["subscribed", "unsubscribed", "pong", "error"],
        "known_event_discriminants": REALTIME_DISCRIMINANTS,
        "join_ack": "subscribed is emitted only after authoritative membership validation and local registration",
        "denied_subscribe": {
            "indistinguishable_cases": [
                "nonexistent_conversation",
                "cross_group_conversation",
                "nonmember",
                "deleted_group"
            ],
            "data_or_error_frame_count": 0,
            "cleanup": "remove every local subscription and socket registry entry before close",
            "close_code": 4001,
            "reason": "membership_required",
            "client_cleanup_class": "evicted"
        },
        "selected_D13_A": {
            "logout_effect": "the short access token remains valid until exp; logout revokes refresh authority only",
            "established_socket_deadline": "bound access-token exp copied from the consumed ticket",
            "deadline_cleanup": "remove every local subscription and registry entry before close",
            "deadline_close_code": 4401,
            "deadline_reason": "realtime_auth_expired",
            "recovery": "refresh_or_reauthenticate_then_new_ticket_then_delta_first"
        },
        "heartbeat": {
            "client_ping_interval_seconds": 25,
            "pong_deadline_seconds": 10,
            "timeout_action": "reconnect_then_delta"
        },
        "unknown_event": {
            "cursor_action": "unchanged",
            "next_action": "request_S1_after_last_applied_cursor"
        },
        "delivery": {
            "duplicates_allowed": true,
            "idempotency_key": "event_id",
            "redis_websocket_role": "acceleration_only",
            "correctness_source": "postgresql_delta_sync"
        }
    })
}
