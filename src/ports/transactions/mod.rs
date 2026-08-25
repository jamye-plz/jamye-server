//! The sole opaque transaction boundary shared by PostgreSQL use cases.

use std::{any::Any, error::Error, fmt, future::Future, pin::Pin};

pub type BoxTransactionHandle = Box<dyn TransactionHandle>;
pub type TransactionFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, TransactionError>> + Send + 'a>>;

pub trait TransactionHandle: Send {
    fn as_any_mut(&mut self) -> &mut (dyn Any + Send);
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send>;
}

pub trait TransactionManager: Send + Sync {
    fn begin(&self) -> TransactionFuture<'_, BoxTransactionHandle>;

    fn commit<'a>(&'a self, handle: BoxTransactionHandle) -> TransactionFuture<'a, ()>;

    fn rollback<'a>(&'a self, handle: BoxTransactionHandle) -> TransactionFuture<'a, ()>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionError;

impl fmt::Display for TransactionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("transaction operation failed")
    }
}

impl Error for TransactionError {}
