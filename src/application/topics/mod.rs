//! Topic lifecycle, cursor-based unread projection, and atomic create use cases.

use std::{collections::HashSet, fmt, sync::Arc};

use sha2::{Digest, Sha256};
use time::{Date, Month};
use uuid::Uuid;

use crate::ports::{
    topics::{
        CreateTopicCommand, CreateTopicOutcome, GetTopicQuery, ListTopicDatesQuery,
        ListTopicMediaQuery, ListTopicTagsQuery, ListTopicsQuery, NewTopicTag, PatchTopicCommand,
        ReplaceTopicTagsCommand, TopicDatePage, TopicMediaPage, TopicNotificationContext,
        TopicPage, TopicRecord, TopicStatus, TopicTagPage, TopicTagSource, TopicsRepository,
        TopicsRepositoryError,
    },
    transactions::{BoxTransactionHandle, TransactionHandle, TransactionManager},
};

pub const DEFAULT_TOPIC_PAGE_LIMIT: u32 = 20;
pub const DEFAULT_DATE_PAGE_LIMIT: u32 = 31;
pub const DEFAULT_TAG_PAGE_LIMIT: u32 = 50;
pub const DEFAULT_MEDIA_PAGE_LIMIT: u32 = 20;
pub const MAX_PAGE_LIMIT: u32 = 100;
pub const MAX_DATE_PAGE_LIMIT: u32 = 366;

#[derive(Clone)]
pub struct TopicsService {
    dependencies: TopicsDependencies,
}

#[derive(Clone)]
pub struct TopicsDependencies {
    pub transactions: Arc<dyn TransactionManager>,
    pub repository: Arc<dyn TopicsRepository>,
}

impl TopicsService {
    pub fn new(dependencies: TopicsDependencies) -> Self {
        Self { dependencies }
    }

    pub async fn create_topic(
        &self,
        author_id: Uuid,
        group_id: Uuid,
        input: TopicCreateInput,
    ) -> Result<CreateTopicOutcome, TopicsError> {
        let command = self.prepare_create_topic(author_id, group_id, input)?;
        let mut transaction = self.begin().await?;
        let result = self
            .create_topic_command_in_transaction(transaction.as_mut(), &command)
            .await;
        self.finish(transaction, result).await
    }

    /// Applies the complete task-7 core create using the caller's transaction.
    ///
    /// Task-12 appends notification operations to this same handle before its
    /// single commit. This method never begins, commits, or rolls back.
    pub async fn create_topic_in_transaction(
        &self,
        transaction: &mut dyn TransactionHandle,
        author_id: Uuid,
        group_id: Uuid,
        input: TopicCreateInput,
    ) -> Result<CreateTopicOutcome, TopicsError> {
        let command = self.prepare_create_topic(author_id, group_id, input)?;
        self.create_topic_command_in_transaction(transaction, &command)
            .await
    }

    /// Validates mounted or standalone input before a caller opens its
    /// transaction, keeping the feature's command construction in one place.
    pub(crate) fn prepare_create_topic(
        &self,
        author_id: Uuid,
        group_id: Uuid,
        input: TopicCreateInput,
    ) -> Result<CreateTopicCommand, TopicsError> {
        create_topic_command(author_id, group_id, input)
    }

    /// Persists a validated topic command on the caller-owned Task-4a handle.
    /// The caller retains commit/rollback ownership.
    pub(crate) async fn create_topic_command_in_transaction(
        &self,
        transaction: &mut dyn TransactionHandle,
        command: &CreateTopicCommand,
    ) -> Result<CreateTopicOutcome, TopicsError> {
        self.dependencies
            .repository
            .create_topic(transaction, command)
            .await
            .map_err(TopicsError::from)
    }

    /// Runs a prepared topic command and reads its canonical topic-created
    /// identity on the caller-owned transaction.
    pub(crate) async fn create_topic_command_with_notification_context_in_transaction(
        &self,
        transaction: &mut dyn TransactionHandle,
        command: &CreateTopicCommand,
    ) -> Result<(CreateTopicOutcome, TopicNotificationContext), TopicsError> {
        let outcome = self
            .create_topic_command_in_transaction(transaction, command)
            .await?;
        let context = self
            .dependencies
            .repository
            .notification_context(transaction, outcome.topic())
            .await
            .map_err(TopicsError::from)?;
        Ok((outcome, context))
    }

    pub async fn list_topics(
        &self,
        actor_id: Uuid,
        group_id: Uuid,
        input: TopicPageInput,
    ) -> Result<TopicPage, TopicsError> {
        let limit = validate_limit(input.limit, DEFAULT_TOPIC_PAGE_LIMIT, MAX_PAGE_LIMIT)?;
        let after = parse_uuid_cursor(input.after)?;
        let date = input.date.map(|value| parse_date(&value)).transpose()?;
        self.dependencies
            .repository
            .list_topics(ListTopicsQuery {
                group_id,
                actor_id,
                after,
                limit,
                date,
            })
            .await
            .map_err(TopicsError::from)
    }

    pub async fn list_topic_dates(
        &self,
        actor_id: Uuid,
        group_id: Uuid,
        input: TopicDatePageInput,
    ) -> Result<TopicDatePage, TopicsError> {
        let limit = validate_limit(input.limit, DEFAULT_DATE_PAGE_LIMIT, MAX_DATE_PAGE_LIMIT)?;
        let after = input.after.map(|value| parse_date(&value)).transpose()?;
        self.dependencies
            .repository
            .list_topic_dates(ListTopicDatesQuery {
                group_id,
                actor_id,
                after,
                limit,
            })
            .await
            .map_err(TopicsError::from)
    }

    pub async fn get_topic(
        &self,
        actor_id: Uuid,
        group_id: Uuid,
        topic_id: Uuid,
    ) -> Result<TopicRecord, TopicsError> {
        self.dependencies
            .repository
            .get_topic(GetTopicQuery {
                group_id,
                topic_id,
                actor_id,
            })
            .await
            .map_err(TopicsError::from)
    }

    pub async fn patch_topic(
        &self,
        actor_id: Uuid,
        group_id: Uuid,
        topic_id: Uuid,
        input: TopicPatchInput,
    ) -> Result<TopicRecord, TopicsError> {
        let command = patch_command(actor_id, group_id, topic_id, input)?;
        let mut transaction = self.begin().await?;
        let result = self
            .dependencies
            .repository
            .patch_topic(transaction.as_mut(), &command)
            .await
            .map_err(TopicsError::from);
        self.finish(transaction, result).await
    }

    pub async fn replace_tags(
        &self,
        actor_id: Uuid,
        group_id: Uuid,
        topic_id: Uuid,
        input: TopicTagsInput,
    ) -> Result<TopicTagPage, TopicsError> {
        let command = tags_command(actor_id, group_id, topic_id, input)?;
        let mut transaction = self.begin().await?;
        let result = self
            .dependencies
            .repository
            .replace_tags(transaction.as_mut(), &command)
            .await
            .map_err(TopicsError::from);
        self.finish(transaction, result).await
    }

    pub async fn list_tags(
        &self,
        actor_id: Uuid,
        group_id: Uuid,
        topic_id: Uuid,
        input: TopicTagPageInput,
    ) -> Result<TopicTagPage, TopicsError> {
        let limit = validate_limit(input.limit, DEFAULT_TAG_PAGE_LIMIT, MAX_PAGE_LIMIT)?;
        let after = parse_uuid_cursor(input.after)?;
        self.dependencies
            .repository
            .list_tags(ListTopicTagsQuery {
                group_id,
                topic_id,
                actor_id,
                after,
                limit,
            })
            .await
            .map_err(TopicsError::from)
    }

    pub async fn list_media(
        &self,
        actor_id: Uuid,
        topic_id: Uuid,
        input: TopicMediaPageInput,
    ) -> Result<TopicMediaPage, TopicsError> {
        let limit = validate_limit(input.limit, DEFAULT_MEDIA_PAGE_LIMIT, MAX_PAGE_LIMIT)?;
        let after = parse_uuid_cursor(input.after)?;
        self.dependencies
            .repository
            .list_media(ListTopicMediaQuery {
                topic_id,
                actor_id,
                after,
                limit,
            })
            .await
            .map_err(TopicsError::from)
    }

    /// Idempotently promotes a seed topic inside a caller-owned transaction.
    pub async fn promote_enriched(
        &self,
        transaction: &mut dyn TransactionHandle,
        topic_id: Uuid,
    ) -> Result<TopicStatus, TopicsError> {
        self.dependencies
            .repository
            .promote_enriched(transaction, topic_id)
            .await
            .map_err(TopicsError::from)
    }

    async fn begin(&self) -> Result<BoxTransactionHandle, TopicsError> {
        self.dependencies
            .transactions
            .begin()
            .await
            .map_err(|_| TopicsError::DatabaseUnavailable)
    }

    async fn finish<T>(
        &self,
        transaction: BoxTransactionHandle,
        result: Result<T, TopicsError>,
    ) -> Result<T, TopicsError> {
        match result {
            Ok(value) => {
                self.dependencies
                    .transactions
                    .commit(transaction)
                    .await
                    .map_err(|_| TopicsError::DatabaseUnavailable)?;
                Ok(value)
            }
            Err(error) => {
                self.dependencies
                    .transactions
                    .rollback(transaction)
                    .await
                    .map_err(|_| TopicsError::DatabaseUnavailable)?;
                Err(error)
            }
        }
    }
}

pub(crate) fn create_topic_command(
    author_id: Uuid,
    group_id: Uuid,
    input: TopicCreateInput,
) -> Result<CreateTopicCommand, TopicsError> {
    let title = normalize_title(input.title)?;
    let topic_id = Uuid::new_v4();
    Ok(CreateTopicCommand {
        topic_id,
        topic_chatroom_id: Uuid::new_v4(),
        topic_event_id: Uuid::new_v4(),
        topic_outbox_id: Uuid::new_v4(),
        author_read_marker_id: Uuid::new_v4(),
        announcement_message_id: Uuid::new_v4(),
        announcement_client_msg_id: Uuid::new_v4(),
        announcement_event_id: Uuid::new_v4(),
        announcement_outbox_id: Uuid::new_v4(),
        group_id,
        author_id,
        idempotency_key: input.idempotency_key,
        request_fingerprint: fingerprint(&title),
        announcement_body: announcement_body(group_id, topic_id, &title),
        title,
    })
}

fn patch_command(
    actor_id: Uuid,
    group_id: Uuid,
    topic_id: Uuid,
    input: TopicPatchInput,
) -> Result<PatchTopicCommand, TopicsError> {
    let title = input.title.map(normalize_title).transpose()?;
    if input.body.as_ref().is_some_and(String::is_empty) {
        return Err(TopicsError::RequestValidation);
    }
    Ok(PatchTopicCommand {
        group_id,
        topic_id,
        actor_id,
        title,
        body: input.body,
    })
}

fn tags_command(
    actor_id: Uuid,
    group_id: Uuid,
    topic_id: Uuid,
    input: TopicTagsInput,
) -> Result<ReplaceTopicTagsCommand, TopicsError> {
    let mut seen = HashSet::with_capacity(input.tags.len());
    let mut tags = Vec::with_capacity(input.tags.len());
    for input in input.tags {
        let tag = input.tag.trim().to_owned();
        if tag.is_empty() || tag.chars().count() > 64 || !seen.insert(tag.clone()) {
            return Err(TopicsError::RequestValidation);
        }
        if input
            .confidence
            .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
        {
            return Err(TopicsError::RequestValidation);
        }
        let source = TopicTagSource::parse(&input.source).ok_or(TopicsError::RequestValidation)?;
        tags.push(NewTopicTag {
            id: Uuid::new_v4(),
            tag,
            source,
            confidence: input.confidence,
        });
    }
    tags.sort_by(|left, right| left.tag.cmp(&right.tag));
    Ok(ReplaceTopicTagsCommand {
        group_id,
        topic_id,
        actor_id,
        tags,
    })
}

fn normalize_title(title: String) -> Result<String, TopicsError> {
    let title = title.trim().to_owned();
    if title.is_empty() || title.chars().count() > 256 {
        return Err(TopicsError::RequestValidation);
    }
    Ok(title)
}

fn validate_limit(limit: Option<u32>, default: u32, maximum: u32) -> Result<u32, TopicsError> {
    let limit = limit.unwrap_or(default);
    if !(1..=maximum).contains(&limit) {
        return Err(TopicsError::RequestValidation);
    }
    Ok(limit)
}

fn parse_uuid_cursor(value: Option<String>) -> Result<Option<Uuid>, TopicsError> {
    value
        .map(|value| Uuid::try_parse(&value))
        .transpose()
        .map_err(|_| TopicsError::RequestValidation)
}

fn parse_date(value: &str) -> Result<Date, TopicsError> {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| index != 4 && index != 7 && !byte.is_ascii_digit())
    {
        return Err(TopicsError::RequestValidation);
    }
    let year = i32::from(bytes[0] - b'0') * 1_000
        + i32::from(bytes[1] - b'0') * 100
        + i32::from(bytes[2] - b'0') * 10
        + i32::from(bytes[3] - b'0');
    let month = (bytes[5] - b'0') * 10 + (bytes[6] - b'0');
    let month = Month::try_from(month)
        .ok()
        .ok_or(TopicsError::RequestValidation)?;
    let day = (bytes[8] - b'0') * 10 + (bytes[9] - b'0');
    Date::from_calendar_date(year, month, day).map_err(|_| TopicsError::RequestValidation)
}

fn fingerprint(title: &str) -> String {
    encode_hex(&Sha256::digest(title.as_bytes()))
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn announcement_body(group_id: Uuid, topic_id: Uuid, title: &str) -> String {
    let mut escaped = title.to_owned();
    for character in ['\\', '[', ']', '(', ')'] {
        escaped = escaped.replace(character, &format!("\\{character}"));
    }
    format!("새로운 주제를 올렸어요: [{escaped}](/groups/{group_id}/topics/{topic_id}/chat)")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopicCreateInput {
    pub idempotency_key: Uuid,
    pub title: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopicPatchInput {
    pub title: Option<String>,
    pub body: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopicPageInput {
    pub after: Option<String>,
    pub limit: Option<u32>,
    pub date: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopicDatePageInput {
    pub after: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopicTagsInput {
    pub tags: Vec<TopicTagInput>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopicTagInput {
    pub tag: String,
    pub source: String,
    pub confidence: Option<f64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopicTagPageInput {
    pub after: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopicMediaPageInput {
    pub after: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TopicsError {
    RequestValidation,
    GroupNotFound,
    MembershipRequired,
    TopicNotFound,
    AuthorRequired,
    TopicManageRequired,
    IdempotencyConflict,
    DatabaseUnavailable,
}

impl From<TopicsRepositoryError> for TopicsError {
    fn from(error: TopicsRepositoryError) -> Self {
        match error {
            TopicsRepositoryError::GroupNotFound => Self::GroupNotFound,
            TopicsRepositoryError::MembershipRequired => Self::MembershipRequired,
            TopicsRepositoryError::TopicNotFound => Self::TopicNotFound,
            TopicsRepositoryError::AuthorRequired => Self::AuthorRequired,
            TopicsRepositoryError::TopicManageRequired => Self::TopicManageRequired,
            TopicsRepositoryError::IdempotencyConflict => Self::IdempotencyConflict,
            TopicsRepositoryError::InvalidData | TopicsRepositoryError::Unavailable => {
                Self::DatabaseUnavailable
            }
        }
    }
}

impl fmt::Display for TopicsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("topic operation failed")
    }
}

impl std::error::Error for TopicsError {}
