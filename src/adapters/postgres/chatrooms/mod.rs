//! PostgreSQL chatroom list, denormalized history, and read-marker adapter.

mod mutation;
mod query;

use sqlx::PgPool;

use crate::{
    adapters::postgres::transactions::connection,
    ports::{
        chatrooms::{
            ChatroomPage, ChatroomsRepository, ChatroomsRepositoryError, ChatroomsRepositoryFuture,
            ListChatroomsQuery, MarkReadCommand, MessageHistoryPage, MessageHistoryQuery,
            ReadMarker, ReadMarkerQuery,
        },
        transactions::TransactionHandle,
    },
};

#[derive(Clone)]
pub struct PostgresChatroomsRepository {
    pool: PgPool,
}

impl PostgresChatroomsRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl ChatroomsRepository for PostgresChatroomsRepository {
    fn list_chatrooms(
        &self,
        query: ListChatroomsQuery,
    ) -> ChatroomsRepositoryFuture<'_, ChatroomPage> {
        Box::pin(query::list_chatrooms(&self.pool, query))
    }

    fn message_history(
        &self,
        query: MessageHistoryQuery,
    ) -> ChatroomsRepositoryFuture<'_, MessageHistoryPage> {
        Box::pin(query::message_history(&self.pool, query))
    }

    fn mark_read<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a MarkReadCommand,
    ) -> ChatroomsRepositoryFuture<'a, ReadMarker> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| ChatroomsRepositoryError::InvalidData)?;
            mutation::mark_read(connection, command).await
        })
    }

    fn read_marker(
        &self,
        query: ReadMarkerQuery,
    ) -> ChatroomsRepositoryFuture<'_, Option<ReadMarker>> {
        Box::pin(query::read_marker(&self.pool, query))
    }
}

pub(super) fn database_error(
    operation: &'static str,
    error: sqlx::Error,
) -> ChatroomsRepositoryError {
    if let sqlx::Error::Database(database) = &error {
        match database.constraint() {
            Some("chatroom_reads_cursor_check") => {
                return ChatroomsRepositoryError::InvalidData;
            }
            Some("uq_chatroom_reads_user_chatroom") => {
                return ChatroomsRepositoryError::Unavailable;
            }
            _ => {}
        }
    }
    tracing::warn!(
        dependency = "postgres",
        failure_kind = "chatrooms",
        operation,
        "PostgreSQL chatroom operation failed"
    );
    ChatroomsRepositoryError::Unavailable
}
