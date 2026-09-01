//! Topic lifecycle, timeline, unread, tag, and atomic-create persistence boundary.

use std::{fmt, future::Future, pin::Pin};

use time::{Date, OffsetDateTime};
use uuid::Uuid;

use crate::ports::transactions::TransactionHandle;

pub type TopicsRepositoryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, TopicsRepositoryError>> + Send + 'a>>;

pub trait TopicsRepository: Send + Sync {
    fn create_topic<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a CreateTopicCommand,
    ) -> TopicsRepositoryFuture<'a, CreateTopicOutcome>;

    fn patch_topic<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a PatchTopicCommand,
    ) -> TopicsRepositoryFuture<'a, TopicRecord>;

    fn promote_enriched<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        topic_id: Uuid,
    ) -> TopicsRepositoryFuture<'a, TopicStatus>;

    fn replace_tags<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a ReplaceTopicTagsCommand,
    ) -> TopicsRepositoryFuture<'a, TopicTagPage>;

    fn list_topics(&self, query: ListTopicsQuery) -> TopicsRepositoryFuture<'_, TopicPage>;

    fn list_topic_dates(
        &self,
        query: ListTopicDatesQuery,
    ) -> TopicsRepositoryFuture<'_, TopicDatePage>;

    fn get_topic(&self, query: GetTopicQuery) -> TopicsRepositoryFuture<'_, TopicRecord>;

    fn list_tags(&self, query: ListTopicTagsQuery) -> TopicsRepositoryFuture<'_, TopicTagPage>;

    fn list_media(&self, query: ListTopicMediaQuery) -> TopicsRepositoryFuture<'_, TopicMediaPage>;

    /// Resolves the immutable canonical `topic.created` identity from the
    /// persisted topic before the caller-owned transaction is committed.
    fn notification_context<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _topic: &'a TopicRecord,
    ) -> TopicsRepositoryFuture<'a, TopicNotificationContext> {
        Box::pin(async { Err(TopicsRepositoryError::Unavailable) })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateTopicCommand {
    pub topic_id: Uuid,
    pub topic_chatroom_id: Uuid,
    pub topic_event_id: Uuid,
    pub topic_outbox_id: Uuid,
    pub author_read_marker_id: Uuid,
    pub announcement_message_id: Uuid,
    pub announcement_client_msg_id: Uuid,
    pub announcement_event_id: Uuid,
    pub announcement_outbox_id: Uuid,
    pub group_id: Uuid,
    pub author_id: Uuid,
    pub idempotency_key: Uuid,
    pub request_fingerprint: String,
    pub title: String,
    pub announcement_body: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CreateTopicOutcome {
    Created(TopicRecord),
    Existing(TopicRecord),
}

impl CreateTopicOutcome {
    pub fn topic(&self) -> &TopicRecord {
        match self {
            Self::Created(topic) | Self::Existing(topic) => topic,
        }
    }

    pub fn into_topic(self) -> TopicRecord {
        match self {
            Self::Created(topic) | Self::Existing(topic) => topic,
        }
    }

    pub fn was_created(&self) -> bool {
        matches!(self, Self::Created(_))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatchTopicCommand {
    pub group_id: Uuid,
    pub topic_id: Uuid,
    pub actor_id: Uuid,
    pub title: Option<String>,
    pub body: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReplaceTopicTagsCommand {
    pub group_id: Uuid,
    pub topic_id: Uuid,
    pub actor_id: Uuid,
    pub tags: Vec<NewTopicTag>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NewTopicTag {
    pub id: Uuid,
    pub tag: String,
    pub source: TopicTagSource,
    pub confidence: Option<f64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ListTopicsQuery {
    pub group_id: Uuid,
    pub actor_id: Uuid,
    pub after: Option<Uuid>,
    pub limit: u32,
    pub date: Option<Date>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ListTopicDatesQuery {
    pub group_id: Uuid,
    pub actor_id: Uuid,
    pub after: Option<Date>,
    pub limit: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GetTopicQuery {
    pub group_id: Uuid,
    pub topic_id: Uuid,
    pub actor_id: Uuid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ListTopicTagsQuery {
    pub group_id: Uuid,
    pub topic_id: Uuid,
    pub actor_id: Uuid,
    pub after: Option<Uuid>,
    pub limit: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ListTopicMediaQuery {
    pub topic_id: Uuid,
    pub actor_id: Uuid,
    pub after: Option<Uuid>,
    pub limit: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TopicStatus {
    Seed,
    Enriched,
}

impl TopicStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Seed => "seed",
            Self::Enriched => "enriched",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "seed" => Some(Self::Seed),
            "enriched" => Some(Self::Enriched),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TopicTagSource {
    Ai,
    User,
}

impl TopicTagSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ai => "ai",
            Self::User => "user",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ai" => Some(Self::Ai),
            "user" => Some(Self::User),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopicRecord {
    pub id: Uuid,
    pub group_id: Uuid,
    pub author_id: Uuid,
    pub author_nickname: String,
    pub author_avatar_url: Option<String>,
    pub title: String,
    pub body: Option<String>,
    pub status: TopicStatus,
    pub tags: Vec<TopicTagRecord>,
    pub media: Vec<TopicMediaRecord>,
    pub chatroom_id: Uuid,
    pub unread: bool,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopicNotificationContext {
    pub group_id: Uuid,
    pub topic_id: Uuid,
    pub conversation_id: Uuid,
    pub source_event_id: Uuid,
    pub author_id: Uuid,
    pub author_display_name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopicTagRecord {
    pub id: Uuid,
    pub topic_id: Uuid,
    pub tag: String,
    pub source: TopicTagSource,
    pub confidence: Option<f64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopicMediaRecord {
    pub id: Uuid,
    pub topic_id: Uuid,
    pub media_upload_id: Uuid,
    pub content_type: String,
    pub object_key: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub byte_size: Option<i64>,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopicPage {
    pub items: Vec<TopicRecord>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopicDatePage {
    pub dates: Vec<String>,
    pub today: String,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopicTagPage {
    pub items: Vec<TopicTagRecord>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopicMediaPage {
    pub items: Vec<TopicMediaRecord>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TopicsRepositoryError {
    GroupNotFound,
    MembershipRequired,
    TopicNotFound,
    AuthorRequired,
    TopicManageRequired,
    IdempotencyConflict,
    InvalidData,
    Unavailable,
}

impl fmt::Display for TopicsRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("topic persistence operation failed")
    }
}

impl std::error::Error for TopicsRepositoryError {}
