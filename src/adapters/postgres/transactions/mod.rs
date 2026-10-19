//! SQLx implementation of the one shared opaque transaction boundary.

use std::any::Any;

use sqlx::{PgConnection, PgPool, Postgres, Transaction};

use crate::ports::transactions::{
    BoxTransactionHandle, TransactionError, TransactionFuture, TransactionHandle,
    TransactionManager,
};

#[derive(Clone)]
pub struct SqlxTransactionManager {
    pool: PgPool,
}

impl SqlxTransactionManager {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl TransactionManager for SqlxTransactionManager {
    fn begin(&self) -> TransactionFuture<'_, BoxTransactionHandle> {
        Box::pin(async move {
            self.pool
                .begin()
                .await
                .map(|transaction| {
                    Box::new(SqlxTransactionHandle::new(transaction)) as BoxTransactionHandle
                })
                .map_err(|error| {
                    log_failure("begin", &error);
                    TransactionError
                })
        })
    }

    fn commit<'a>(&'a self, handle: BoxTransactionHandle) -> TransactionFuture<'a, ()> {
        Box::pin(async move {
            into_transaction(handle)?.commit().await.map_err(|error| {
                log_failure("commit", &error);
                TransactionError
            })
        })
    }

    fn rollback<'a>(&'a self, handle: BoxTransactionHandle) -> TransactionFuture<'a, ()> {
        Box::pin(async move {
            into_transaction(handle)?.rollback().await.map_err(|error| {
                log_failure("rollback", &error);
                TransactionError
            })
        })
    }
}

struct SqlxTransactionHandle {
    transaction: Option<Transaction<'static, Postgres>>,
}

impl SqlxTransactionHandle {
    fn new(transaction: Transaction<'static, Postgres>) -> Self {
        Self {
            transaction: Some(transaction),
        }
    }
}

impl TransactionHandle for SqlxTransactionHandle {
    fn as_any_mut(&mut self) -> &mut (dyn Any + Send) {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }
}

pub(crate) fn connection(
    handle: &mut dyn TransactionHandle,
) -> Result<&mut PgConnection, TransactionError> {
    let handle = handle
        .as_any_mut()
        .downcast_mut::<SqlxTransactionHandle>()
        .ok_or(TransactionError)?;
    let transaction = handle.transaction.as_mut().ok_or(TransactionError)?;
    Ok(&mut **transaction)
}

fn into_transaction(
    handle: BoxTransactionHandle,
) -> Result<Transaction<'static, Postgres>, TransactionError> {
    let mut handle = handle
        .into_any()
        .downcast::<SqlxTransactionHandle>()
        .map_err(|_| TransactionError)?;
    handle.transaction.take().ok_or(TransactionError)
}

fn log_failure(operation: &'static str, error: &sqlx::Error) {
    let sqlstate = crate::adapters::postgres::sqlstate(error);
    tracing::warn!(
        dependency = "postgres",
        failure_kind = "transaction",
        operation,
        sqlstate = sqlstate.as_deref().unwrap_or("unavailable"),
        "PostgreSQL transaction operation failed"
    );
}
