//! Membership-safe chatroom queries, denormalized history, and read-marker use cases.

use std::{fmt, sync::Arc};

use uuid::Uuid;

use crate::ports::{
    chatrooms::{
        ChatroomPage, ChatroomsRepository, ChatroomsRepositoryError, ListChatroomsQuery,
        MarkReadCommand, MessageHistoryPage, MessageHistoryQuery, ReadMarker, ReadMarkerQuery,
    },
    transactions::{BoxTransactionHandle, TransactionHandle, TransactionManager},
};

pub const DEFAULT_PAGE_LIMIT: u32 = 50;
pub const MAX_PAGE_LIMIT: u32 = 100;

#[derive(Clone)]
pub struct ChatroomsService {
    transactions: Arc<dyn TransactionManager>,
    repository: Arc<dyn ChatroomsRepository>,
}

impl ChatroomsService {
    pub fn new(
        transactions: Arc<dyn TransactionManager>,
        repository: Arc<dyn ChatroomsRepository>,
    ) -> Self {
        Self {
            transactions,
            repository,
        }
    }

    pub async fn list_chatrooms(
        &self,
        user_id: Uuid,
        group_id: Uuid,
        input: ChatroomPageInput,
    ) -> Result<ChatroomPage, ChatroomsError> {
        let (after, limit) = validate_page(input.after, input.limit)?;
        self.repository
            .list_chatrooms(ListChatroomsQuery {
                group_id,
                user_id,
                after,
                limit,
            })
            .await
            .map_err(ChatroomsError::from)
    }

    pub async fn message_history(
        &self,
        user_id: Uuid,
        chatroom_id: Uuid,
        input: HistoryPageInput,
    ) -> Result<MessageHistoryPage, ChatroomsError> {
        let (before, limit) = validate_page(input.before, input.limit)?;
        self.repository
            .message_history(MessageHistoryQuery {
                chatroom_id,
                user_id,
                before,
                limit,
            })
            .await
            .map_err(ChatroomsError::from)
    }

    pub async fn mark_read(
        &self,
        user_id: Uuid,
        chatroom_id: Uuid,
        input: ReadCursorInput,
    ) -> Result<ReadMarker, ChatroomsError> {
        let cursor = validate_read_cursor(&input.cursor)?;
        let mut transaction = self.begin().await?;
        let result = self
            .mark_read_in_transaction(transaction.as_mut(), user_id, chatroom_id, cursor)
            .await;
        self.finish(transaction, result).await
    }

    pub async fn mark_read_in_transaction(
        &self,
        transaction: &mut dyn TransactionHandle,
        user_id: Uuid,
        chatroom_id: Uuid,
        cursor: i64,
    ) -> Result<ReadMarker, ChatroomsError> {
        if cursor <= 0 {
            return Err(ChatroomsError::RequestValidation);
        }
        self.repository
            .mark_read(
                transaction,
                &MarkReadCommand {
                    marker_id: Uuid::new_v4(),
                    user_id,
                    chatroom_id,
                    cursor,
                },
            )
            .await
            .map_err(ChatroomsError::from)
    }

    pub async fn read_marker(
        &self,
        user_id: Uuid,
        chatroom_id: Uuid,
    ) -> Result<Option<ReadMarker>, ChatroomsError> {
        self.repository
            .read_marker(ReadMarkerQuery {
                user_id,
                chatroom_id,
            })
            .await
            .map_err(ChatroomsError::from)
    }

    async fn begin(&self) -> Result<BoxTransactionHandle, ChatroomsError> {
        self.transactions
            .begin()
            .await
            .map_err(|_| ChatroomsError::DatabaseUnavailable)
    }

    async fn finish(
        &self,
        transaction: BoxTransactionHandle,
        result: Result<ReadMarker, ChatroomsError>,
    ) -> Result<ReadMarker, ChatroomsError> {
        match result {
            Ok(marker) => {
                self.transactions
                    .commit(transaction)
                    .await
                    .map_err(|_| ChatroomsError::DatabaseUnavailable)?;
                Ok(marker)
            }
            Err(error) => {
                self.transactions
                    .rollback(transaction)
                    .await
                    .map_err(|_| ChatroomsError::DatabaseUnavailable)?;
                Err(error)
            }
        }
    }
}

fn validate_page(
    cursor: Option<String>,
    limit: Option<u32>,
) -> Result<(Option<Uuid>, u32), ChatroomsError> {
    let cursor = cursor
        .map(|value| Uuid::try_parse(&value))
        .transpose()
        .map_err(|_| ChatroomsError::RequestValidation)?;
    let limit = limit.unwrap_or(DEFAULT_PAGE_LIMIT);
    if limit == 0 || limit > MAX_PAGE_LIMIT {
        return Err(ChatroomsError::RequestValidation);
    }
    Ok((cursor, limit))
}

fn validate_read_cursor(cursor: &str) -> Result<i64, ChatroomsError> {
    let cursor = cursor
        .parse::<i64>()
        .map_err(|_| ChatroomsError::RequestValidation)?;
    if cursor <= 0 {
        return Err(ChatroomsError::RequestValidation);
    }
    Ok(cursor)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatroomPageInput {
    pub after: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryPageInput {
    pub before: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadCursorInput {
    pub cursor: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChatroomsError {
    RequestValidation,
    MembershipRequired,
    DatabaseUnavailable,
}

impl From<ChatroomsRepositoryError> for ChatroomsError {
    fn from(error: ChatroomsRepositoryError) -> Self {
        match error {
            ChatroomsRepositoryError::MembershipRequired => Self::MembershipRequired,
            ChatroomsRepositoryError::CursorInvalid => Self::RequestValidation,
            ChatroomsRepositoryError::InvalidData | ChatroomsRepositoryError::Unavailable => {
                Self::DatabaseUnavailable
            }
        }
    }
}

impl fmt::Display for ChatroomsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("chatroom request failed")
    }
}

impl std::error::Error for ChatroomsError {}
