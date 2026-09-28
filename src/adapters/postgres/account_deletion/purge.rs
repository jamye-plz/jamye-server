use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;

use crate::ports::account_deletion::{
    AccountDeletionRepositoryError, AccountPurgeClaim, AccountPurgeClaimRequest,
};

use super::database_error;

type PurgeClaimRow = (Uuid, String, i64, time::OffsetDateTime);

pub(super) async fn claim_account_purges(
    pool: &PgPool,
    request: AccountPurgeClaimRequest,
) -> Result<Vec<AccountPurgeClaim>, AccountDeletionRepositoryError> {
    validate_request(&request)?;
    let batch_size = i64::from(request.batch_size);
    let grace_days = i64::from(request.grace_days);
    let lease_milliseconds = duration_milliseconds(request.lease_duration)?;
    let rows = sqlx::query_as::<_, PurgeClaimRow>(
        "WITH server_clock AS MATERIALIZED (SELECT clock_timestamp() AS now), \
         candidates AS ( \
             SELECT account.id \
             FROM users account, server_clock clock \
             WHERE account.deleted_at IS NOT NULL \
               AND account.deleted_at <= clock.now - ($3::BIGINT * INTERVAL '1 day') \
               AND ( \
                   account.account_purge_claim_owner IS NULL \
                   OR account.account_purge_claim_expires_at <= clock.now \
               ) \
             ORDER BY account.deleted_at, account.id \
             FOR UPDATE OF account SKIP LOCKED \
             LIMIT $2 \
         ), claimed AS ( \
             UPDATE users account \
             SET account_purge_claim_owner = $1, \
                 account_purge_claim_generation = account.account_purge_claim_generation + 1, \
                 account_purge_claim_expires_at = (SELECT now FROM server_clock) \
                     + ($4::BIGINT * INTERVAL '1 millisecond') \
             FROM candidates \
             WHERE account.id = candidates.id \
             RETURNING account.id, account.account_purge_claim_owner, \
                       account.account_purge_claim_generation, \
                       account.account_purge_claim_expires_at \
         ) \
         SELECT id, account_purge_claim_owner, account_purge_claim_generation, \
                account_purge_claim_expires_at \
         FROM claimed \
         ORDER BY account_purge_claim_expires_at, id",
    )
    .bind(&request.claim_owner)
    .bind(batch_size)
    .bind(grace_days)
    .bind(lease_milliseconds)
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("account_purge_claim", error))?;
    rows.into_iter().map(claim_from_row).collect()
}

pub(super) async fn release_account_purge_claim(
    pool: &PgPool,
    claim: &AccountPurgeClaim,
) -> Result<bool, AccountDeletionRepositoryError> {
    let result = sqlx::query(
        "UPDATE users \
         SET account_purge_claim_owner = NULL, \
             account_purge_claim_expires_at = NULL \
         WHERE id = $1 \
           AND account_purge_claim_owner = $2 \
           AND account_purge_claim_generation = $3 \
           AND account_purge_claim_expires_at = $4",
    )
    .bind(claim.user_id)
    .bind(&claim.claim_owner)
    .bind(claim.claim_generation)
    .bind(claim.claim_expires_at)
    .execute(pool)
    .await
    .map_err(|error| database_error("account_purge_release", error))?;
    Ok(result.rows_affected() == 1)
}

fn validate_request(
    request: &AccountPurgeClaimRequest,
) -> Result<(), AccountDeletionRepositoryError> {
    if request.claim_owner.is_empty()
        || request.claim_owner.trim() != request.claim_owner
        || request.claim_owner.chars().count() > 128
        || request.claim_owner.chars().any(char::is_control)
        || request.grace_days == 0
        || request.batch_size == 0
        || request.lease_duration.is_zero()
    {
        return Err(AccountDeletionRepositoryError::InvalidData);
    }
    Ok(())
}

fn duration_milliseconds(duration: Duration) -> Result<i64, AccountDeletionRepositoryError> {
    i64::try_from(duration.as_millis()).map_err(|_| AccountDeletionRepositoryError::InvalidData)
}

fn claim_from_row(row: PurgeClaimRow) -> Result<AccountPurgeClaim, AccountDeletionRepositoryError> {
    if row.1.is_empty() || row.2 <= 0 {
        return Err(AccountDeletionRepositoryError::InvalidData);
    }
    Ok(AccountPurgeClaim {
        user_id: row.0,
        claim_owner: row.1,
        claim_generation: row.2,
        claim_expires_at: row.3,
    })
}
