//! Chatroom list, denormalized history, and monotonic read-marker persistence boundary.

use std::{fmt, future::Future, pin::Pin};

use time::OffsetDateTime;
use uuid::Uuid;

use crate::{domain::messaging::CanonicalMessage, ports::transactions::TransactionHandle};

pub type ChatroomsRepositoryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ChatroomsRepositoryError>> + Send + 'a>>;

pub trait ChatroomsRepository: Send + Sync {
    fn list_chatrooms(
        &self,
        query: ListChatroomsQuery,
    ) -> ChatroomsRepositoryFuture<'_, ChatroomPage>;

    fn message_history(
        &self,
        query: MessageHistoryQuery,
    ) -> ChatroomsRepositoryFuture<'_, MessageHistoryPage>;

    fn mark_read<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a MarkReadCommand,
    ) -> ChatroomsRepositoryFuture<'a, ReadMarker>;

    fn read_marker(
        &self,
        query: ReadMarkerQuery,
    ) -> ChatroomsRepositoryFuture<'_, Option<ReadMarker>>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ListChatroomsQuery {
    pub group_id: Uuid,
    pub user_id: Uuid,
    pub after: Option<Uuid>,
    pub limit: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MessageHistoryQuery {
    pub chatroom_id: Uuid,
    pub user_id: Uuid,
    pub before: Option<Uuid>,
    pub limit: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MarkReadCommand {
    pub marker_id: Uuid,
    pub user_id: Uuid,
    pub chatroom_id: Uuid,
    pub cursor: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadMarkerQuery {
    pub user_id: Uuid,
    pub chatroom_id: Uuid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChatroomKind {
    Main,
    Topic,
}

impl ChatroomKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Topic => "topic",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "main" => Some(Self::Main),
            "topic" => Some(Self::Topic),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatroomRecord {
    pub id: Uuid,
    pub group_id: Uuid,
    pub chatroom_type: ChatroomKind,
    pub topic_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatroomPage {
    pub items: Vec<ChatroomRecord>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageHistoryRecord {
    pub message: CanonicalMessage,
    pub sender_nickname: Option<String>,
    pub sender_avatar_url: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageHistoryPage {
    pub items: Vec<MessageHistoryRecord>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadMarker {
    pub id: Uuid,
    pub user_id: Uuid,
    pub chatroom_id: Uuid,
    pub last_read_cursor: i64,
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChatroomsRepositoryError {
    MembershipRequired,
    CursorInvalid,
    InvalidData,
    Unavailable,
}

impl fmt::Display for ChatroomsRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("chatroom persistence operation failed")
    }
}

impl std::error::Error for ChatroomsRepositoryError {}
