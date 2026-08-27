//! PostgreSQL notification history and single-read adapter.

mod events;
mod mutation;
mod query;

use sqlx::PgPool;

use crate::ports::{
    push::{
        ClearTopicNotificationsCommand, ListNotificationsQuery, MarkNotificationReadCommand,
        NotificationClearReport, NotificationEventsRepository, NotificationEventsRepositoryFuture,
        NotificationFanoutReport, NotificationPage, NotificationReadRecord,
        NotificationsRepository, NotificationsRepositoryError, NotificationsRepositoryFuture,
        RecordMessageNotificationCommand, RecordTopicNotificationCommand,
    },
    transactions::TransactionHandle,
};

use super::transactions::connection;

#[derive(Clone)]
pub struct PostgresNotificationsRepository {
    pool: PgPool,
}

impl PostgresNotificationsRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl NotificationsRepository for PostgresNotificationsRepository {
    fn list_notifications(
        &self,
        query: ListNotificationsQuery,
    ) -> NotificationsRepositoryFuture<'_, NotificationPage> {
        Box::pin(query::list_notifications(&self.pool, query))
    }

    fn mark_notification_read<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a MarkNotificationReadCommand,
    ) -> NotificationsRepositoryFuture<'a, NotificationReadRecord> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| NotificationsRepositoryError::InvalidData)?;
            mutation::mark_notification_read(connection, command).await
        })
    }
}

impl NotificationEventsRepository for PostgresNotificationsRepository {
    fn record_topic_created<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a RecordTopicNotificationCommand,
    ) -> NotificationEventsRepositoryFuture<'a, NotificationFanoutReport> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| NotificationsRepositoryError::InvalidData)?;
            events::record_topic_created(connection, command).await
        })
    }

    fn record_message_created<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a RecordMessageNotificationCommand,
    ) -> NotificationEventsRepositoryFuture<'a, NotificationFanoutReport> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| NotificationsRepositoryError::InvalidData)?;
            events::record_message_created(connection, command).await
        })
    }

    fn clear_topic_notifications<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a ClearTopicNotificationsCommand,
    ) -> NotificationEventsRepositoryFuture<'a, NotificationClearReport> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| NotificationsRepositoryError::InvalidData)?;
            events::clear_topic_notifications(connection, command).await
        })
    }
}

pub(super) fn database_error(
    operation: &'static str,
    _error: sqlx::Error,
) -> NotificationsRepositoryError {
    tracing::warn!(
        dependency = "postgres",
        failure_kind = "notifications",
        operation,
        "PostgreSQL notification operation failed"
    );
    NotificationsRepositoryError::Unavailable
}
