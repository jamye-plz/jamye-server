//! PostgreSQL storage for internal realtime revocation controls and final authorization.

use std::{collections::HashSet, error::Error, fmt, time::Duration};

use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    adapters::postgres::transactions::connection,
    application::realtime::membership_revocation::{
        ControlIntentAppender, ControlIntentError, ControlIntentFuture, REALTIME_CONTROL_VERSION,
        RealtimeControlIntent,
    },
    ports::transactions::TransactionHandle,
};

type ControlClaimRow = (
    Uuid,
    String,
    i16,
    String,
    Uuid,
    Value,
    String,
    i64,
    OffsetDateTime,
    i32,
);

#[derive(Clone)]
pub struct PostgresRealtimeRevocations {
    pool: PgPool,
}

impl PostgresRealtimeRevocations {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn claim_controls(
        &self,
        request: &ControlClaimRequest,
    ) -> Result<Vec<ClaimedControlIntent>, RealtimeRevocationStoreError> {
        if request.claim_owner.is_empty()
            || request.claim_owner.len() > 128
            || request.batch_size == 0
        {
            return Err(RealtimeRevocationStoreError::InvalidData);
        }
        let batch_size = i64::from(request.batch_size);
        let lease_milliseconds = duration_milliseconds(request.lease_duration)?;
        let rows = sqlx::query_as::<_, ControlClaimRow>(
            "WITH server_clock AS MATERIALIZED ( \
                 SELECT clock_timestamp() AS now \
             ), candidates AS ( \
                 SELECT o.id \
                 FROM outbox_events o, server_clock c \
                 WHERE o.intent_type = 'control' \
                   AND ( \
                       (o.status = 'pending' \
                        AND COALESCE(o.next_attempt_at, o.created_at) <= c.now) \
                       OR (o.status = 'claimed' AND o.claim_expires_at <= c.now) \
                   ) \
                 ORDER BY COALESCE(o.next_attempt_at, o.created_at), o.created_at, o.id \
                 FOR UPDATE OF o SKIP LOCKED \
                 LIMIT $1 \
             ), claimed AS ( \
                 UPDATE outbox_events o \
                 SET status = 'claimed', \
                     claim_owner = $2, \
                     claim_generation = o.claim_generation + 1, \
                     claim_expires_at = (SELECT now FROM server_clock) \
                         + ($3::BIGINT * INTERVAL '1 millisecond'), \
                     next_attempt_at = NULL \
                 FROM candidates \
                 WHERE o.id = candidates.id \
                 RETURNING o.id, o.event_type, o.event_version, o.aggregate_type, \
                           o.aggregate_id, o.payload, o.claim_owner, o.claim_generation, \
                           o.claim_expires_at, o.attempt_count \
             ) \
             SELECT id, event_type, event_version, aggregate_type, aggregate_id, payload, \
                    claim_owner, claim_generation, claim_expires_at, attempt_count \
             FROM claimed \
             ORDER BY claim_expires_at, id",
        )
        .bind(batch_size)
        .bind(&request.claim_owner)
        .bind(lease_milliseconds)
        .fetch_all(&self.pool)
        .await
        .map_err(|_| database_error("claim_controls"))?;

        rows.into_iter().map(control_claim_from_row).collect()
    }

    pub async fn mark_published(
        &self,
        claim: &ClaimedControlIntent,
    ) -> Result<bool, RealtimeRevocationStoreError> {
        let result = sqlx::query(
            "UPDATE outbox_events \
             SET status = 'published', \
                 claim_owner = NULL, \
                 claim_expires_at = NULL, \
                 published_at = clock_timestamp(), \
                 last_error_code = NULL \
             WHERE id = $1 \
               AND intent_type = 'control' \
               AND status = 'claimed' \
               AND claim_owner = $2 \
               AND claim_generation = $3 \
               AND claim_expires_at > clock_timestamp()",
        )
        .bind(claim.id)
        .bind(&claim.claim_owner)
        .bind(claim.claim_generation)
        .execute(&self.pool)
        .await
        .map_err(|_| database_error("mark_control_published"))?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn record_failure(
        &self,
        claim: &ClaimedControlIntent,
        code: &'static str,
        retry_delay: Duration,
        max_attempts: u32,
    ) -> Result<ControlFailureDisposition, RealtimeRevocationStoreError> {
        let retry_milliseconds = duration_milliseconds(retry_delay)?;
        let max_attempts =
            i32::try_from(max_attempts).map_err(|_| RealtimeRevocationStoreError::InvalidData)?;
        let status = sqlx::query_scalar::<_, String>(
            "WITH server_clock AS MATERIALIZED ( \
                 SELECT clock_timestamp() AS now \
             ) \
             UPDATE outbox_events \
             SET attempt_count = attempt_count + 1, \
                 status = CASE \
                     WHEN attempt_count + 1 >= $6 \
                       OR (deadline_at IS NOT NULL \
                           AND deadline_at <= (SELECT now FROM server_clock) \
                               + ($5::BIGINT * INTERVAL '1 millisecond')) \
                     THEN 'dead_letter' \
                     ELSE 'pending' \
                 END, \
                 claim_owner = NULL, \
                 claim_expires_at = NULL, \
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
               AND intent_type = 'control' \
               AND status = 'claimed' \
               AND claim_owner = $2 \
               AND claim_generation = $3 \
               AND claim_expires_at > (SELECT now FROM server_clock) \
             RETURNING status",
        )
        .bind(claim.id)
        .bind(&claim.claim_owner)
        .bind(claim.claim_generation)
        .bind(code)
        .bind(retry_milliseconds)
        .bind(max_attempts)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| database_error("record_control_failure"))?;

        match status.as_deref() {
            Some("pending") => Ok(ControlFailureDisposition::RetryScheduled),
            Some("dead_letter") => Ok(ControlFailureDisposition::DeadLettered),
            None => Ok(ControlFailureDisposition::StaleClaim),
            Some(_) => Err(RealtimeRevocationStoreError::InvalidData),
        }
    }

    pub async fn conversation_ids(
        &self,
        group_id: Uuid,
    ) -> Result<Vec<Uuid>, RealtimeRevocationStoreError> {
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM chatrooms WHERE group_id = $1 ORDER BY id")
            .bind(group_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|_| database_error("revocation_conversations"))
    }

    pub async fn authorized_users(
        &self,
        conversation_id: Uuid,
        candidate_user_ids: &[Uuid],
    ) -> Result<HashSet<Uuid>, RealtimeRevocationStoreError> {
        if candidate_user_ids.is_empty() {
            return Ok(HashSet::new());
        }
        let candidates = candidate_user_ids.to_vec();
        let rows = sqlx::query_scalar::<_, Uuid>(
            "SELECT DISTINCT m.user_id \
             FROM chatrooms c \
             JOIN groups g ON g.id = c.group_id AND g.deleted_at IS NULL \
             JOIN memberships m ON m.group_id = g.id \
             WHERE c.id = $1 AND m.user_id = ANY($2::UUID[])",
        )
        .bind(conversation_id)
        .bind(candidates)
        .fetch_all(&self.pool)
        .await
        .map_err(|_| database_error("authorize_delivery_batch"))?;
        Ok(rows.into_iter().collect())
    }
}

impl ControlIntentAppender for PostgresRealtimeRevocations {
    fn append<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        intent: &'a RealtimeControlIntent,
    ) -> ControlIntentFuture<'a> {
        Box::pin(async move {
            if intent.version() != REALTIME_CONTROL_VERSION {
                return Err(ControlIntentError::InvalidData);
            }
            let payload =
                serde_json::to_value(intent).map_err(|_| ControlIntentError::InvalidData)?;
            let connection =
                connection(transaction).map_err(|_| ControlIntentError::InvalidData)?;
            sqlx::query(
                "INSERT INTO outbox_events \
                 (id, intent_type, event_type, event_version, aggregate_type, aggregate_id, \
                  conversation_event_id, payload) \
                 VALUES ($1, 'control', $2, $3, $4, $5, NULL, $6)",
            )
            .bind(intent.control_id())
            .bind(intent.event_type())
            .bind(intent.version())
            .bind(intent.aggregate_type())
            .bind(intent.aggregate_id())
            .bind(payload)
            .execute(connection)
            .await
            .map(|_| ())
            .map_err(|_| {
                log_database_failure("append_control");
                ControlIntentError::Unavailable
            })
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlClaimRequest {
    pub claim_owner: String,
    pub batch_size: u32,
    pub lease_duration: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimedControlIntent {
    pub id: Uuid,
    pub intent: RealtimeControlIntent,
    pub claim_owner: String,
    pub claim_generation: i64,
    pub claim_expires_at: OffsetDateTime,
    pub attempt_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlFailureDisposition {
    RetryScheduled,
    DeadLettered,
    StaleClaim,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RealtimeRevocationStoreError {
    Unavailable,
    InvalidData,
}

impl fmt::Display for RealtimeRevocationStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("realtime revocation persistence operation failed")
    }
}

impl Error for RealtimeRevocationStoreError {}

fn control_claim_from_row(
    row: ControlClaimRow,
) -> Result<ClaimedControlIntent, RealtimeRevocationStoreError> {
    let intent = serde_json::from_value::<RealtimeControlIntent>(row.5)
        .map_err(|_| RealtimeRevocationStoreError::InvalidData)?;
    if row.1 != intent.event_type()
        || row.2 != intent.version()
        || row.3 != intent.aggregate_type()
        || row.4 != intent.aggregate_id()
        || row.0 != intent.control_id()
    {
        return Err(RealtimeRevocationStoreError::InvalidData);
    }
    let attempt_count =
        u32::try_from(row.9).map_err(|_| RealtimeRevocationStoreError::InvalidData)?;
    Ok(ClaimedControlIntent {
        id: row.0,
        intent,
        claim_owner: row.6,
        claim_generation: row.7,
        claim_expires_at: row.8,
        attempt_count,
    })
}

fn duration_milliseconds(duration: Duration) -> Result<i64, RealtimeRevocationStoreError> {
    i64::try_from(duration.as_millis()).map_err(|_| RealtimeRevocationStoreError::InvalidData)
}

fn database_error(operation: &'static str) -> RealtimeRevocationStoreError {
    log_database_failure(operation);
    RealtimeRevocationStoreError::Unavailable
}

fn log_database_failure(operation: &'static str) {
    tracing::warn!(
        dependency = "postgres",
        failure_kind = "realtime_revocation",
        operation,
        "PostgreSQL realtime revocation operation failed"
    );
}
