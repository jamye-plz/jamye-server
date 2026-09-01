//! PostgreSQL messaging repository.

mod delta;
mod send;

use sqlx::PgPool;

use crate::{
    adapters::postgres::transactions::connection,
    domain::messaging::{EventPage, SendMessageCommand},
    ports::{
        messaging::{
            DeltaQuery, MessageDeliveryContext, MessagingFuture, MessagingRepository,
            MessagingRepositoryError, PersistMessageOutcome, PersistedMessage,
        },
        transactions::TransactionHandle,
    },
};

#[derive(Clone)]
pub struct PostgresMessagingRepository {
    pool: PgPool,
}

impl PostgresMessagingRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl MessagingRepository for PostgresMessagingRepository {
    fn send<'a>(
        &'a self,
        handle: &'a mut dyn TransactionHandle,
        command: &'a SendMessageCommand,
    ) -> MessagingFuture<'a, PersistMessageOutcome> {
        Box::pin(send::persist(handle, command))
    }

    fn events(&self, query: DeltaQuery) -> MessagingFuture<'_, EventPage> {
        Box::pin(delta::page(&self.pool, query))
    }

    fn delivery_context<'a>(
        &'a self,
        handle: &'a mut dyn TransactionHandle,
        message: &'a PersistedMessage,
    ) -> MessagingFuture<'a, MessageDeliveryContext> {
        Box::pin(async move {
            let connection =
                connection(handle).map_err(|_| MessagingRepositoryError::DatabaseUnavailable)?;
            send::delivery_context(connection, message).await
        })
    }
}
