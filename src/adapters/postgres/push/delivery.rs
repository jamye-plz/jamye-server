use std::time::Duration;

use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::ports::push::{
    ClaimedPushDelivery, PushDeliveryClaim, PushDeliveryClaimRequest, PushDeliveryFailureCode,
    PushDeliveryFailureDisposition, PushRepositoryError,
};

use super::database_error;

const MAX_CLAIM_OWNER_CHARS: usize = 128;

type DeliveryClaimRow = (Uuid, String, i64, OffsetDateTime, i32);

pub(super) async fn claim_deliveries(
    pool: &PgPool,
    request: PushDeliveryClaimRequest,
) -> Result<Vec<ClaimedPushDelivery>, PushRepositoryError> {
    let lease_milliseconds = validate_claim_request(&request)?;
    let batch_size = i64::from(request.batch_size);
    let rows = sqlx::query_as::<_, DeliveryClaimRow>(
        "WITH server_clock AS MATERIALIZED ( \
             SELECT clock_timestamp() AS now \
         ), candidates AS ( \
             SELECT occurrence.id \
             FROM push_delivery_intents occurrence, server_clock clock \
             WHERE ( \
                 (occurrence.status IN ('pending', 'retryable') \
                  AND COALESCE(occurrence.next_attempt_at, occurrence.created_at) <= clock.now) \
                 OR (occurrence.status = 'claimed' \
                     AND occurrence.lease_expires_at <= clock.now) \
             ) \
             ORDER BY COALESCE(occurrence.next_attempt_at, \
                               occurrence.lease_expires_at, occurrence.created_at), \
                      occurrence.created_at, occurrence.id \
             FOR UPDATE OF occurrence SKIP LOCKED \
             LIMIT $1 \
         ), claimed AS ( \
             UPDATE push_delivery_intents occurrence \
             SET status = 'claimed', \
                 claim_owner = $2, \
                 claim_generation = occurrence.claim_generation + 1, \
                 lease_expires_at = (SELECT now FROM server_clock) \
                     + ($3::BIGINT * INTERVAL '1 millisecond'), \
                 next_attempt_at = NULL \
             FROM candidates \
             WHERE occurrence.id = candidates.id \
             RETURNING occurrence.id, occurrence.claim_owner, \
                       occurrence.claim_generation, occurrence.lease_expires_at, \
                       occurrence.attempt_count \
         ) \
         SELECT id, claim_owner, claim_generation, lease_expires_at, attempt_count \
         FROM claimed \
         ORDER BY lease_expires_at, id",
    )
    .bind(batch_size)
    .bind(&request.claim_owner)
    .bind(lease_milliseconds)
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("push_delivery_claim", error))?;

    rows.into_iter().map(delivery_claim_from_row).collect()
}

pub(super) async fn mark_delivery_succeeded(
    pool: &PgPool,
    claim: &ClaimedPushDelivery,
) -> Result<bool, PushRepositoryError> {
    validate_claim(&claim.claim)?;
    let result = sqlx::query(
        "WITH server_clock AS MATERIALIZED ( \
             SELECT clock_timestamp() AS now \
         ) \
         UPDATE push_delivery_intents \
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
    .bind(claim.claim.occurrence_id)
    .bind(&claim.claim.claim_owner)
    .bind(claim.claim.claim_generation)
    .execute(pool)
    .await
    .map_err(|error| database_error("push_delivery_succeeded", error))?;
    Ok(result.rows_affected() == 1)
}

pub(super) async fn record_delivery_failure(
    pool: &PgPool,
    claim: &ClaimedPushDelivery,
    code: PushDeliveryFailureCode,
    retry_delay: Duration,
    max_attempts: u32,
) -> Result<PushDeliveryFailureDisposition, PushRepositoryError> {
    validate_claim(&claim.claim)?;
    let retry_milliseconds = duration_milliseconds(retry_delay)?;
    let max_attempts = i32::try_from(max_attempts).map_err(|_| PushRepositoryError::InvalidData)?;
    if max_attempts == 0 {
        return Err(PushRepositoryError::InvalidData);
    }
    let status = sqlx::query_scalar::<_, String>(
        "WITH server_clock AS MATERIALIZED ( \
             SELECT clock_timestamp() AS now \
         ) \
         UPDATE push_delivery_intents \
         SET attempt_count = attempt_count + 1, \
             status = CASE \
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
                 WHEN attempt_count + 1 >= $6 \
                   OR (deadline_at IS NOT NULL \
                       AND deadline_at <= (SELECT now FROM server_clock) \
                           + ($5::BIGINT * INTERVAL '1 millisecond')) \
                 THEN NULL \
                 ELSE (SELECT now FROM server_clock) \
                     + ($5::BIGINT * INTERVAL '1 millisecond') \
             END, \
             dead_lettered_at = CASE \
                 WHEN attempt_count + 1 >= $6 \
                   OR (deadline_at IS NOT NULL \
                       AND deadline_at <= (SELECT now FROM server_clock) \
                           + ($5::BIGINT * INTERVAL '1 millisecond')) \
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
    .bind(claim.claim.occurrence_id)
    .bind(&claim.claim.claim_owner)
    .bind(claim.claim.claim_generation)
    .bind(code.as_str())
    .bind(retry_milliseconds)
    .bind(max_attempts)
    .fetch_optional(pool)
    .await
    .map_err(|error| database_error("push_delivery_failure", error))?;

    match status.as_deref() {
        Some("retryable") => Ok(PushDeliveryFailureDisposition::RetryScheduled),
        Some("dead_letter") => Ok(PushDeliveryFailureDisposition::DeadLettered),
        None => Ok(PushDeliveryFailureDisposition::StaleClaim),
        Some(_) => Err(PushRepositoryError::InvalidData),
    }
}

fn validate_claim_request(request: &PushDeliveryClaimRequest) -> Result<i64, PushRepositoryError> {
    let owner_length = request.claim_owner.chars().count();
    if owner_length == 0
        || owner_length > MAX_CLAIM_OWNER_CHARS
        || request.claim_owner.chars().any(char::is_control)
        || request.batch_size == 0
    {
        return Err(PushRepositoryError::InvalidData);
    }
    let lease_milliseconds = duration_milliseconds(request.lease_duration)?;
    if lease_milliseconds == 0 {
        return Err(PushRepositoryError::InvalidData);
    }
    Ok(lease_milliseconds)
}

fn validate_claim(claim: &PushDeliveryClaim) -> Result<(), PushRepositoryError> {
    let owner_length = claim.claim_owner.chars().count();
    if claim.claim_generation <= 0
        || owner_length == 0
        || owner_length > MAX_CLAIM_OWNER_CHARS
        || claim.claim_owner.chars().any(char::is_control)
    {
        return Err(PushRepositoryError::InvalidData);
    }
    Ok(())
}

fn delivery_claim_from_row(
    row: DeliveryClaimRow,
) -> Result<ClaimedPushDelivery, PushRepositoryError> {
    let attempt_count = u32::try_from(row.4).map_err(|_| PushRepositoryError::InvalidData)?;
    Ok(ClaimedPushDelivery {
        claim: PushDeliveryClaim {
            occurrence_id: row.0,
            claim_owner: row.1,
            claim_generation: row.2,
        },
        claim_expires_at: row.3,
        attempt_count,
    })
}

fn duration_milliseconds(duration: Duration) -> Result<i64, PushRepositoryError> {
    i64::try_from(duration.as_millis()).map_err(|_| PushRepositoryError::InvalidData)
}
