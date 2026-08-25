//! Public C0 wire DTOs.
//!
//! These types describe transport data only. They intentionally do not reuse
//! domain entities or add runtime feature implementations.

// The generator reflects these DTOs through utoipa/schemars; runtime code does
// not construct every enum variant at C0.
#![allow(dead_code)]

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    pub request_id: Uuid,
    #[schema(required = true, nullable)]
    pub details: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MediaRef {
    /// A finalized, unconsumed upload capability owned by the authenticated user.
    pub media_upload_id: Uuid,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MessageCreate {
    /// The stable, app-generated idempotency key reused for every retry.
    pub client_msg_id: Uuid,
    /// Missing, null, and empty are allowed only when media is non-empty.
    pub body: Option<String>,
    #[schema(max_items = 4)]
    #[schemars(length(max = 4))]
    pub media: Option<Vec<MediaRef>>,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    User,
    System,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MessageAttachment {
    pub id: Uuid,
    pub media_upload_id: Uuid,
    #[serde(rename = "type")]
    pub content_type: String,
    pub byte_size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration: Option<u64>,
    pub filename: Option<String>,
    pub position: u8,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonicalMessage {
    pub id: Uuid,
    pub chatroom_id: Uuid,
    pub sender_id: Option<Uuid>,
    pub client_msg_id: Option<Uuid>,
    pub body: Option<String>,
    #[serde(rename = "type")]
    pub message_type: MessageKind,
    /// UTC ISO 8601 timestamp.
    pub created_at: String,
    pub media: Vec<MessageAttachment>,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileScope {
    ChatHistory,
    GroupTopics,
    Notifications,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UnsupportedEventMarker {
    pub event_id: Uuid,
    pub cursor: String,
    pub reconcile_scope: ReconcileScope,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize, ToSchema)]
pub enum MessageCreatedType {
    #[serde(rename = "message.created")]
    MessageCreated,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MessageCreatedEvent {
    #[schemars(range(min = 1, max = 1))]
    #[schema(minimum = 1, maximum = 1)]
    pub version: u8,
    #[serde(rename = "type")]
    pub event_type: MessageCreatedType,
    pub event_id: Uuid,
    pub conversation_id: Uuid,
    pub cursor: String,
    /// UTC ISO 8601 timestamp.
    pub occurred_at: String,
    pub data: CanonicalMessage,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize, ToSchema)]
#[serde(untagged)]
pub enum DeltaItem {
    Known(MessageCreatedEvent),
    Unsupported(UnsupportedEventMarker),
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EventPage {
    pub items: Vec<DeltaItem>,
    #[schema(required = true, nullable)]
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RealtimeTicket {
    pub ticket: String,
    /// UTC ISO 8601 timestamp bounded by the access-token expiry.
    pub expires_at: String,
    pub contract_version: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    Subscribe {
        request_id: Uuid,
        conversation_id: Uuid,
    },
    Unsubscribe {
        request_id: Uuid,
        conversation_id: Uuid,
    },
    Ping { nonce: String },
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerControlFrame {
    Subscribed {
        request_id: Uuid,
        conversation_id: Uuid,
    },
    Unsubscribed {
        request_id: Uuid,
        conversation_id: Uuid,
    },
    Pong {
        nonce: String,
    },
    Error {
        request_id: Uuid,
        code: String,
        message: String,
    },
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(untagged)]
pub enum ServerFrame {
    Control(ServerControlFrame),
    MessageCreated(MessageCreatedEvent),
}
