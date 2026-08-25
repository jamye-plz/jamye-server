//! PostgreSQL outbox lease state machine and authoritative conversation authorization.

use std::time::Duration;

use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::ports::realtime::{
    ClaimedOutboxEvent, ConversationAuthorizer, FailureDisposition, OutboxClaimRequest,
    OutboxRepository, PublishFailureCode, RealtimeFuture, RealtimePortError,
};

type ClaimRow = (
    Uuid,
    Uuid,
    Option<Uuid>,
    Value,
    String,
    i64,
    OffsetDateTime,
    i32,
);

#[derive(Clone)]
pub struct PostgresRealtimeRepository {
    pool: PgPool,
}

impl PostgresRealtimeRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn claim_events(
        &self,
        request: OutboxClaimRequest,
    ) -> Result<Vec<ClaimedOutboxEvent>, RealtimePortError> {
        let batch_size = i64::from(request.batch_size);
        let lease_milliseconds = duration_milliseconds(request.lease_duration)?;
        let rows = sqlx::query_as::<_, ClaimRow>(
            "WITH server_clock AS MATERIALIZED ( \
                 SELECT clock_timestamp() AS now \
             ), candidates AS ( \
                 SELECT o.id \
                 FROM outbox_events o, server_clock c \
                 WHERE o.intent_type = 'conversation' \
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
                 RETURNING o.id, o.aggregate_id, o.conversation_event_id, o.payload, \
                           o.claim_owner, o.claim_generation, o.claim_expires_at, \
                           o.attempt_count \
             ) \
             SELECT id, aggregate_id, conversation_event_id, payload, claim_owner, \
                    claim_generation, claim_expires_at, attempt_count \
             FROM claimed \
             ORDER BY claim_expires_at, id",
        )
        .bind(batch_size)
        .bind(&request.claim_owner)
        .bind(lease_milliseconds)
        .fetch_all(&self.pool)
        .await
        .map_err(|_| database_error("claim"))?;

        rows.into_iter().map(claim_from_row).collect()
    }

    async fn publish_marker(&self, claim: &ClaimedOutboxEvent) -> Result<bool, RealtimePortError> {
        let result = sqlx::query(
            "UPDATE outbox_events \
             SET status = 'published', \
                 claim_owner = NULL, \
                 claim_expires_at = NULL, \
                 published_at = clock_timestamp(), \
                 last_error_code = NULL \
             WHERE id = $1 \
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
        .map_err(|_| database_error("mark_published"))?;
        Ok(result.rows_affected() == 1)
    }

    async fn failure_marker(
        &self,
        claim: &ClaimedOutboxEvent,
        code: PublishFailureCode,
        retry_delay: Duration,
        max_attempts: u32,
    ) -> Result<FailureDisposition, RealtimePortError> {
        let retry_milliseconds = duration_milliseconds(retry_delay)?;
        let max_attempts =
            i32::try_from(max_attempts).map_err(|_| RealtimePortError::InvalidData)?;
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
               AND status = 'claimed' \
               AND claim_owner = $2 \
               AND claim_generation = $3 \
               AND claim_expires_at > (SELECT now FROM server_clock) \
             RETURNING status",
        )
        .bind(claim.id)
        .bind(&claim.claim_owner)
        .bind(claim.claim_generation)
        .bind(code.as_str())
        .bind(retry_milliseconds)
        .bind(max_attempts)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| database_error("record_failure"))?;

        match status.as_deref() {
            Some("pending") => Ok(FailureDisposition::RetryScheduled),
            Some("dead_letter") => Ok(FailureDisposition::DeadLettered),
            None => Ok(FailureDisposition::StaleClaim),
            Some(_) => Err(RealtimePortError::InvalidData),
        }
    }

    async fn authorization(
        &self,
        user_id: Uuid,
        conversation_id: Uuid,
    ) -> Result<bool, RealtimePortError> {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS ( \
                 SELECT 1 \
                 FROM chatrooms c \
                 JOIN groups g ON g.id = c.group_id AND g.deleted_at IS NULL \
                 JOIN memberships m ON m.group_id = g.id AND m.user_id = $2 \
                 WHERE c.id = $1 \
             )",
        )
        .bind(conversation_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|_| database_error("authorize_subscription"))
    }
}

impl OutboxRepository for PostgresRealtimeRepository {
    fn claim(&self, request: OutboxClaimRequest) -> RealtimeFuture<'_, Vec<ClaimedOutboxEvent>> {
        Box::pin(self.claim_events(request))
    }

    fn mark_published<'a>(&'a self, claim: &'a ClaimedOutboxEvent) -> RealtimeFuture<'a, bool> {
        Box::pin(self.publish_marker(claim))
    }

    fn record_failure<'a>(
        &'a self,
        claim: &'a ClaimedOutboxEvent,
        code: PublishFailureCode,
        retry_delay: Duration,
        max_attempts: u32,
    ) -> RealtimeFuture<'a, FailureDisposition> {
        Box::pin(self.failure_marker(claim, code, retry_delay, max_attempts))
    }
}

impl ConversationAuthorizer for PostgresRealtimeRepository {
    fn is_authorized(&self, user_id: Uuid, conversation_id: Uuid) -> RealtimeFuture<'_, bool> {
        Box::pin(self.authorization(user_id, conversation_id))
    }
}

fn claim_from_row(row: ClaimRow) -> Result<ClaimedOutboxEvent, RealtimePortError> {
    let event_id = row.2.ok_or(RealtimePortError::InvalidData)?;
    let attempt_count = u32::try_from(row.7).map_err(|_| RealtimePortError::InvalidData)?;
    Ok(ClaimedOutboxEvent {
        id: row.0,
        conversation_id: row.1,
        event_id,
        payload: row.3,
        claim_owner: row.4,
        claim_generation: row.5,
        claim_expires_at: row.6,
        attempt_count,
    })
}

fn duration_milliseconds(duration: Duration) -> Result<i64, RealtimePortError> {
    i64::try_from(duration.as_millis()).map_err(|_| RealtimePortError::InvalidData)
}

fn database_error(operation: &'static str) -> RealtimePortError {
    tracing::warn!(
        dependency = "postgres",
        failure_kind = "realtime",
        operation,
        "PostgreSQL realtime operation failed"
    );
    RealtimePortError::Unavailable
}
