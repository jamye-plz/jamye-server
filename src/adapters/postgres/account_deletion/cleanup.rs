//! PostgreSQL durable account-object cleanup transitions.
//!
//! The cleanup queue is authoritative PostgreSQL state. Each claim is one
//! atomic statement transaction: it locks deterministically due work with
//! `SKIP LOCKED`, advances the generation, and installs a PostgreSQL-time
//! lease. Completion calls use that generation and the still-live lease as a
//! compare-and-swap fence, so late workers cannot regress terminal state.

use std::time::Duration;

use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::ports::account_deletion::{
    AccountObjectDeletionClaim, AccountObjectDeletionClaimRequest,
    AccountObjectDeletionFailureCode, AccountObjectDeletionFailureDisposition,
    AccountObjectDeletionRepository, AccountObjectDeletionRepositoryError,
    AccountObjectDeletionRepositoryFuture,
};

const MAX_CLAIM_OWNER_CHARS: usize = 128;

type ObjectDeletionClaimRow = (Uuid, String, String, i64, OffsetDateTime, i32);

impl AccountObjectDeletionRepository for super::PostgresAccountDeletionRepository {
    fn claim_object_deletions(
        &self,
        request: AccountObjectDeletionClaimRequest,
    ) -> AccountObjectDeletionRepositoryFuture<'_, Vec<AccountObjectDeletionClaim>> {
        Box::pin(claim_object_deletions(&self.pool, request))
    }

    fn mark_object_deleted<'a>(
        &'a self,
        claim: &'a AccountObjectDeletionClaim,
    ) -> AccountObjectDeletionRepositoryFuture<'a, bool> {
        Box::pin(mark_object_deleted(&self.pool, claim))
    }

    fn record_object_deletion_failure<'a>(
        &'a self,
        claim: &'a AccountObjectDeletionClaim,
        code: AccountObjectDeletionFailureCode,
        retry_delay: Duration,
        max_attempts: u32,
    ) -> AccountObjectDeletionRepositoryFuture<'a, AccountObjectDeletionFailureDisposition> {
        Box::pin(record_object_deletion_failure(
            &self.pool,
            claim,
            code,
            retry_delay,
            max_attempts,
        ))
    }
}

async fn claim_object_deletions(
    pool: &PgPool,
    request: AccountObjectDeletionClaimRequest,
) -> Result<Vec<AccountObjectDeletionClaim>, AccountObjectDeletionRepositoryError> {
    let lease_milliseconds = validate_claim_request(&request)?;
    let batch_size = i64::from(request.batch_size);
    let rows = sqlx::query_as::<_, ObjectDeletionClaimRow>(
        "WITH server_clock AS MATERIALIZED ( \
             SELECT clock_timestamp() AS now \
         ), candidates AS ( \
             SELECT intent.id \
             FROM account_object_deletion_intents intent, server_clock clock \
             WHERE ( \
                 (intent.status IN ('pending', 'retryable') \
                  AND COALESCE(intent.next_attempt_at, intent.created_at) <= clock.now) \
                 OR (intent.status = 'claimed' \
                     AND intent.lease_expires_at <= clock.now) \
             ) \
             ORDER BY COALESCE(intent.next_attempt_at, intent.lease_expires_at, intent.created_at), \
                      intent.created_at, intent.id \
             FOR UPDATE OF intent SKIP LOCKED \
             LIMIT $1 \
         ), claimed AS ( \
             UPDATE account_object_deletion_intents intent \
             SET status = 'claimed', \
                 claim_owner = $2, \
                 claim_generation = intent.claim_generation + 1, \
                 lease_expires_at = (SELECT now FROM server_clock) \
                     + ($3::BIGINT * INTERVAL '1 millisecond'), \
                 next_attempt_at = NULL \
             FROM candidates \
             WHERE intent.id = candidates.id \
             RETURNING intent.id, intent.object_key, intent.claim_owner, \
                       intent.claim_generation, intent.lease_expires_at, intent.attempt_count \
         ) \
         SELECT id, object_key, claim_owner, claim_generation, lease_expires_at, attempt_count \
         FROM claimed \
         ORDER BY lease_expires_at, id",
    )
    .bind(batch_size)
    .bind(&request.claim_owner)
    .bind(lease_milliseconds)
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("account_object_deletion_claim", error))?;

    rows.into_iter().map(claim_from_row).collect()
}

async fn mark_object_deleted(
    pool: &PgPool,
    claim: &AccountObjectDeletionClaim,
) -> Result<bool, AccountObjectDeletionRepositoryError> {
    validate_claim(claim)?;
    let result = sqlx::query(
        "WITH server_clock AS MATERIALIZED ( \
             SELECT clock_timestamp() AS now \
         ) \
         UPDATE account_object_deletion_intents \
         SET status = 'succeeded', \
             claim_owner = NULL, \
             lease_expires_at = NULL, \
             next_attempt_at = NULL, \
             last_error_code = NULL, \
             succeeded_at = (SELECT now FROM server_clock) \
         WHERE id = $1 \
           AND status = 'claimed' \
           AND claim_owner = $2 \
           AND claim_generation = $3 \
           AND lease_expires_at > (SELECT now FROM server_clock)",
    )
    .bind(claim.intent_id)
    .bind(&claim.claim_owner)
    .bind(claim.claim_generation)
    .execute(pool)
    .await
    .map_err(|error| database_error("account_object_deletion_succeeded", error))?;
    Ok(result.rows_affected() == 1)
}

async fn record_object_deletion_failure(
    pool: &PgPool,
    claim: &AccountObjectDeletionClaim,
    code: AccountObjectDeletionFailureCode,
    retry_delay: Duration,
    max_attempts: u32,
) -> Result<AccountObjectDeletionFailureDisposition, AccountObjectDeletionRepositoryError> {
    validate_claim(claim)?;
    let retry_milliseconds = duration_milliseconds(retry_delay)?;
    if retry_milliseconds == 0 {
        return Err(AccountObjectDeletionRepositoryError::InvalidData);
    }
    let max_attempts = i32::try_from(max_attempts)
        .map_err(|_| AccountObjectDeletionRepositoryError::InvalidData)?;
    if max_attempts == 0 {
        return Err(AccountObjectDeletionRepositoryError::InvalidData);
    }

    let status = sqlx::query_scalar::<_, String>(
        "WITH server_clock AS MATERIALIZED ( \
             SELECT clock_timestamp() AS now \
         ) \
         UPDATE account_object_deletion_intents \
         SET attempt_count = attempt_count + 1, \
             status = CASE \
                 WHEN $4 = 'access_denied' THEN 'failed' \
                 WHEN attempt_count + 1 >= $6 \
                   OR (deadline_at IS NOT NULL \
                       AND deadline_at <= (SELECT now FROM server_clock) \
                           + ($5::BIGINT * INTERVAL '1 millisecond')) \
                 THEN 'dead_letter' \
                 ELSE 'retryable' \
             END, \
             claim_owner = NULL, \
             lease_expires_at = NULL, \
             next_attempt_at = CASE \
                 WHEN $4 = 'access_denied' \
                   OR attempt_count + 1 >= $6 \
                   OR (deadline_at IS NOT NULL \
                       AND deadline_at <= (SELECT now FROM server_clock) \
                           + ($5::BIGINT * INTERVAL '1 millisecond')) \
                 THEN NULL \
                 ELSE (SELECT now FROM server_clock) \
                     + ($5::BIGINT * INTERVAL '1 millisecond') \
             END, \
             failed_at = CASE \
                 WHEN $4 = 'access_denied' THEN (SELECT now FROM server_clock) \
                 ELSE NULL \
             END, \
             dead_lettered_at = CASE \
                 WHEN $4 <> 'access_denied' \
                   AND (attempt_count + 1 >= $6 \
                        OR (deadline_at IS NOT NULL \
                            AND deadline_at <= (SELECT now FROM server_clock) \
                                + ($5::BIGINT * INTERVAL '1 millisecond'))) \
                 THEN (SELECT now FROM server_clock) \
                 ELSE NULL \
             END, \
             last_error_code = $4 \
         WHERE id = $1 \
           AND status = 'claimed' \
           AND claim_owner = $2 \
           AND claim_generation = $3 \
           AND lease_expires_at > (SELECT now FROM server_clock) \
         RETURNING status",
    )
    .bind(claim.intent_id)
    .bind(&claim.claim_owner)
    .bind(claim.claim_generation)
    .bind(code.as_str())
    .bind(retry_milliseconds)
    .bind(max_attempts)
    .fetch_optional(pool)
    .await
    .map_err(|error| database_error("account_object_deletion_failure", error))?;

    match status.as_deref() {
        Some("retryable") => Ok(AccountObjectDeletionFailureDisposition::RetryScheduled),
        Some("failed") => Ok(AccountObjectDeletionFailureDisposition::Failed),
        Some("dead_letter") => Ok(AccountObjectDeletionFailureDisposition::DeadLettered),
        None => Ok(AccountObjectDeletionFailureDisposition::StaleClaim),
        Some(_) => Err(AccountObjectDeletionRepositoryError::InvalidData),
    }
}

fn validate_claim_request(
    request: &AccountObjectDeletionClaimRequest,
) -> Result<i64, AccountObjectDeletionRepositoryError> {
    validate_claim_owner(&request.claim_owner)?;
    if request.batch_size == 0 {
        return Err(AccountObjectDeletionRepositoryError::InvalidData);
    }
    let lease_milliseconds = duration_milliseconds(request.lease_duration)?;
    if lease_milliseconds == 0 {
        return Err(AccountObjectDeletionRepositoryError::InvalidData);
    }
    Ok(lease_milliseconds)
}

fn validate_claim(
    claim: &AccountObjectDeletionClaim,
) -> Result<(), AccountObjectDeletionRepositoryError> {
    if claim.claim_generation <= 0 {
        return Err(AccountObjectDeletionRepositoryError::InvalidData);
    }
    validate_claim_owner(&claim.claim_owner)
}

fn validate_claim_owner(owner: &str) -> Result<(), AccountObjectDeletionRepositoryError> {
    let owner_length = owner.chars().count();
    if owner_length == 0
        || owner_length > MAX_CLAIM_OWNER_CHARS
        || owner.trim() != owner
        || owner.chars().any(char::is_control)
    {
        return Err(AccountObjectDeletionRepositoryError::InvalidData);
    }
    Ok(())
}

fn claim_from_row(
    row: ObjectDeletionClaimRow,
) -> Result<AccountObjectDeletionClaim, AccountObjectDeletionRepositoryError> {
    let attempt_count =
        u32::try_from(row.5).map_err(|_| AccountObjectDeletionRepositoryError::InvalidData)?;
    if row.3 <= 0 {
        return Err(AccountObjectDeletionRepositoryError::InvalidData);
    }
    validate_claim_owner(&row.2)?;
    Ok(AccountObjectDeletionClaim::new(
        row.0,
        row.1,
        row.2,
        row.3,
        row.4,
        attempt_count,
    ))
}

fn duration_milliseconds(duration: Duration) -> Result<i64, AccountObjectDeletionRepositoryError> {
    i64::try_from(duration.as_millis())
        .map_err(|_| AccountObjectDeletionRepositoryError::InvalidData)
}

fn database_error(
    operation: &'static str,
    _error: sqlx::Error,
) -> AccountObjectDeletionRepositoryError {
    tracing::warn!(
        dependency = "postgres",
        failure_kind = "account_object_deletion",
        operation,
        "PostgreSQL account-object cleanup persistence operation failed"
    );
    AccountObjectDeletionRepositoryError::Unavailable
}
