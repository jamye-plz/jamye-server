//! PostgreSQL messaging repository.

mod delta;
mod send;

use sqlx::PgPool;

use crate::{
    domain::messaging::{EventPage, SendMessageCommand},
    ports::{
        messaging::{DeltaQuery, MessagingFuture, MessagingRepository, PersistMessageOutcome},
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
}
