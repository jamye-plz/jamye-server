use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    User,
    System,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalMessage {
    pub id: Uuid,
    pub chatroom_id: Uuid,
    pub sender_id: Option<Uuid>,
    #[serde(default)]
    pub sender_nickname: Option<String>,
    #[serde(default)]
    pub sender_avatar_url: Option<String>,
    pub client_msg_id: Option<Uuid>,
    pub body: Option<String>,
    #[serde(rename = "type")]
    pub message_type: MessageKind,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub media: Vec<MessageAttachment>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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
    /// The `media_uploads` id of this attachment's poster image, when it is a
    /// video that has one bound alongside it. `None` for non-video kinds,
    /// legacy rows, or a poster that failed to bind.
    pub poster_media_id: Option<Uuid>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileScope {
    ChatHistory,
    GroupTopics,
    Notifications,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UnsupportedEventMarker {
    pub event_id: Uuid,
    pub cursor: String,
    pub reconcile_scope: ReconcileScope,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum MessageCreatedType {
    #[serde(rename = "message.created")]
    MessageCreated,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageCreatedEvent {
    pub version: u8,
    #[serde(rename = "type")]
    pub event_type: MessageCreatedType,
    pub event_id: Uuid,
    pub conversation_id: Uuid,
    pub cursor: String,
    #[serde(with = "time::serde::rfc3339")]
    pub occurred_at: OffsetDateTime,
    pub data: CanonicalMessage,
}

// `MessageCreatedEvent` grew alongside `CanonicalMessage`'s new sender
// display fields, so it is now noticeably larger than `UnsupportedEventMarker`.
// Boxing it would ripple through every `DeltaItem::Known(...)` construction
// site across the delta/realtime adapters for a pure size optimization with
// no behavioral upside here, so the lint is deliberately suppressed instead.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum DeltaItem {
    Known(MessageCreatedEvent),
    Unsupported(UnsupportedEventMarker),
}

impl DeltaItem {
    pub fn cursor(&self) -> &str {
        match self {
            Self::Known(event) => &event.cursor,
            Self::Unsupported(marker) => &marker.cursor,
        }
    }

    pub fn event_id(&self) -> Uuid {
        match self {
            Self::Known(event) => event.event_id,
            Self::Unsupported(marker) => marker.event_id,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventPage {
    pub items: Vec<DeltaItem>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SendMessageCommand {
    pub chatroom_id: Uuid,
    pub sender_id: Uuid,
    pub client_msg_id: Uuid,
    pub body: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ConversationEvent {
    pub id: Uuid,
    pub cursor: i64,
    pub conversation_id: Uuid,
    pub event_type: String,
    pub event_version: i16,
    pub payload: Value,
    pub occurred_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::CanonicalMessage;

    /// A stored `conversation_events`/`outbox_events` payload written before
    /// this session's `sender_nickname`/`sender_avatar_url` fields existed
    /// omits both keys entirely. `#[serde(default)]` must let it keep
    /// deserializing under `CanonicalMessage`'s `#[serde(deny_unknown_fields)]`.
    #[test]
    fn legacy_payload_without_sender_display_fields_deserializes_with_none()
    -> Result<(), serde_json::Error> {
        let legacy_payload = serde_json::json!({
            "id": "20000000-0000-4000-8000-000000000001",
            "chatroom_id": "10000000-0000-4000-8000-000000000001",
            "sender_id": "10000000-0000-4000-8000-000000000002",
            "client_msg_id": "30000000-0000-4000-8000-000000000001",
            "body": "legacy stored payload",
            "type": "user",
            "created_at": "2026-08-22T00:00:00Z",
            "media": [],
        });

        let message: CanonicalMessage = serde_json::from_value(legacy_payload)?;

        assert_eq!(message.sender_nickname, None);
        assert_eq!(message.sender_avatar_url, None);
        Ok(())
    }
}
