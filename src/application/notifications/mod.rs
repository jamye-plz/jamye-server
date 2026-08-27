//! Notification history pagination and owner-scoped single-read use cases.

use std::{fmt, sync::Arc};

use uuid::Uuid;

use crate::ports::{
    push::{
        ClearTopicNotificationsCommand, ListNotificationsQuery, MarkNotificationReadCommand,
        NotificationClearReport, NotificationEventsRepository, NotificationFanoutReport,
        NotificationPage, NotificationsRepository, NotificationsRepositoryError,
        RecordMessageNotificationCommand, RecordTopicNotificationCommand,
    },
    transactions::{BoxTransactionHandle, TransactionHandle, TransactionManager},
};

pub const DEFAULT_NOTIFICATION_PAGE_LIMIT: u32 = 50;
pub const MAX_NOTIFICATION_PAGE_LIMIT: u32 = 100;

#[derive(Clone)]
pub struct NotificationsService {
    dependencies: NotificationsDependencies,
}

#[derive(Clone)]
pub struct NotificationsDependencies {
    pub transactions: Arc<dyn TransactionManager>,
    pub repository: Arc<dyn NotificationsRepository>,
}

#[derive(Clone)]
pub struct NotificationTransactionOperations {
    repository: Arc<dyn NotificationEventsRepository>,
}

impl NotificationTransactionOperations {
    pub fn new(repository: Arc<dyn NotificationEventsRepository>) -> Self {
        Self { repository }
    }

    pub async fn record_topic_created(
        &self,
        transaction: &mut dyn TransactionHandle,
        command: &RecordTopicNotificationCommand,
    ) -> Result<NotificationFanoutReport, NotificationsError> {
        self.repository
            .record_topic_created(transaction, command)
            .await
            .map_err(NotificationsError::from)
    }

    pub async fn record_message_created(
        &self,
        transaction: &mut dyn TransactionHandle,
        command: &RecordMessageNotificationCommand,
    ) -> Result<NotificationFanoutReport, NotificationsError> {
        self.repository
            .record_message_created(transaction, command)
            .await
            .map_err(NotificationsError::from)
    }

    pub async fn clear_topic_notifications(
        &self,
        transaction: &mut dyn TransactionHandle,
        command: &ClearTopicNotificationsCommand,
    ) -> Result<NotificationClearReport, NotificationsError> {
        self.repository
            .clear_topic_notifications(transaction, command)
            .await
            .map_err(NotificationsError::from)
    }
}

impl NotificationsService {
    pub fn new(dependencies: NotificationsDependencies) -> Self {
        Self { dependencies }
    }

    pub async fn list_notifications(
        &self,
        user_id: Uuid,
        input: NotificationPageInput,
    ) -> Result<NotificationPage, NotificationsError> {
        let (after, limit) = validate_page(input)?;
        self.dependencies
            .repository
            .list_notifications(ListNotificationsQuery {
                user_id,
                after,
                limit,
            })
            .await
            .map_err(NotificationsError::from)
    }

    pub async fn mark_read(
        &self,
        user_id: Uuid,
        notification_id: Uuid,
    ) -> Result<(), NotificationsError> {
        let mut transaction = self.begin().await?;
        let result = self
            .dependencies
            .repository
            .mark_notification_read(
                transaction.as_mut(),
                &MarkNotificationReadCommand {
                    user_id,
                    notification_id,
                },
            )
            .await
            .map(|_| ())
            .map_err(NotificationsError::from);
        self.finish(transaction, result).await
    }

    async fn begin(&self) -> Result<BoxTransactionHandle, NotificationsError> {
        self.dependencies
            .transactions
            .begin()
            .await
            .map_err(|_| NotificationsError::DatabaseUnavailable)
    }

    async fn finish<T>(
        &self,
        transaction: BoxTransactionHandle,
        result: Result<T, NotificationsError>,
    ) -> Result<T, NotificationsError> {
        match result {
            Ok(value) => {
                self.dependencies
                    .transactions
                    .commit(transaction)
                    .await
                    .map_err(|_| NotificationsError::DatabaseUnavailable)?;
                Ok(value)
            }
            Err(error) => {
                self.dependencies
                    .transactions
                    .rollback(transaction)
                    .await
                    .map_err(|_| NotificationsError::DatabaseUnavailable)?;
                Err(error)
            }
        }
    }
}

fn validate_page(input: NotificationPageInput) -> Result<(Option<Uuid>, u32), NotificationsError> {
    let after = input
        .after
        .map(|value| Uuid::try_parse(&value))
        .transpose()
        .map_err(|_| NotificationsError::RequestValidation)?;
    let limit = input.limit.unwrap_or(DEFAULT_NOTIFICATION_PAGE_LIMIT);
    if !(1..=MAX_NOTIFICATION_PAGE_LIMIT).contains(&limit) {
        return Err(NotificationsError::RequestValidation);
    }
    Ok((after, limit))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationPageInput {
    pub after: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationsError {
    RequestValidation,
    NotificationNotFound,
    DatabaseUnavailable,
}

impl From<NotificationsRepositoryError> for NotificationsError {
    fn from(error: NotificationsRepositoryError) -> Self {
        match error {
            NotificationsRepositoryError::NotificationNotFound => Self::NotificationNotFound,
            NotificationsRepositoryError::CursorInvalid => Self::RequestValidation,
            NotificationsRepositoryError::InvalidData
            | NotificationsRepositoryError::Unavailable => Self::DatabaseUnavailable,
        }
    }
}

impl fmt::Display for NotificationsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("notification operation failed")
    }
}

impl std::error::Error for NotificationsError {}
