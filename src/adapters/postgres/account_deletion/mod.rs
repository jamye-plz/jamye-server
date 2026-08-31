//! PostgreSQL account-deletion persistence adapter.
//!
//! The application service owns the transaction and invokes this adapter in
//! two phases. Preparation acquires the shared lock order before it hands live
//! memberships to Task-6/Task-9. Finalization only performs the
//! account-deletion-specific transition on that same transaction.

pub mod cleanup;

mod payload_scrub;
mod preparation;
mod transition;

use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    adapters::postgres::transactions::connection,
    ports::{
        account_deletion::{
            AccountDeletionPreparation, AccountDeletionReport, AccountDeletionRepository,
            AccountDeletionRepositoryError, AccountDeletionRepositoryFuture,
        },
        transactions::TransactionHandle,
    },
};

use preparation::prepare_deletion;
use transition::finalize_deletion;

#[derive(Clone)]
pub struct PostgresAccountDeletionRepository {
    pool: PgPool,
}

impl PostgresAccountDeletionRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl AccountDeletionRepository for PostgresAccountDeletionRepository {
    fn prepare_deletion<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        user_id: Uuid,
    ) -> AccountDeletionRepositoryFuture<'a, AccountDeletionPreparation> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| AccountDeletionRepositoryError::InvalidData)?;
            prepare_deletion(connection, user_id).await
        })
    }

    fn finalize_deletion<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        user_id: Uuid,
    ) -> AccountDeletionRepositoryFuture<'a, AccountDeletionReport> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| AccountDeletionRepositoryError::InvalidData)?;
            finalize_deletion(connection, user_id).await
        })
    }
}

pub(super) fn database_error(
    operation: &'static str,
    _error: sqlx::Error,
) -> AccountDeletionRepositoryError {
    tracing::warn!(
        dependency = "postgres",
        failure_kind = "account_deletion",
        operation,
        "PostgreSQL account-deletion persistence operation failed"
    );
    AccountDeletionRepositoryError::Unavailable
}
