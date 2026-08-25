use std::{sync::Arc, time::Duration};

use jamye_server::{
    adapters::postgres::realtime::PostgresRealtimeRepository,
    application::realtime::{OutboxWorker, OutboxWorkerConfig, OutboxWorkerError},
    ports::realtime::{
        ClaimedOutboxEvent, ConversationAuthorizer, FailureDisposition, OutboxClaimRequest,
        OutboxRepository, PublishFailureCode, RealtimeEventPublisher, RealtimeFuture,
    },
};
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::Barrier;
use uuid::Uuid;

use crate::{TestResult, fixture_support::insert_owner_fixture, postgres_support::TestDatabase};

#[test]
fn publish_timeout_plus_safety_margin_must_be_strictly_below_the_lease() {
    let repository = Arc::new(NoopRepository);
    let publisher = Arc::new(NoopPublisher);
    let mut config = worker_config();
    config.lease_duration = Duration::from_millis(300);
    assert!(matches!(
        OutboxWorker::new(repository.clone(), publisher.clone(), config.clone()),
        Err(OutboxWorkerError::InvalidConfiguration)
    ));
    config.lease_duration = Duration::from_millis(301);
    assert!(OutboxWorker::new(repository, publisher, config).is_ok());
}

#[tokio::test]
async fn every_claimed_publish_starts_within_the_same_lease_budget() -> TestResult {
    let repository = Arc::new(BatchRepository);
    let publisher = Arc::new(BarrierPublisher(Barrier::new(2)));
    let mut config = worker_config();
    config.batch_size = 2;
    config.lease_duration = Duration::from_millis(301);
    let worker = OutboxWorker::new(repository, publisher, config)?;

    let report = worker.run_once().await?;
    assert_eq!(report.claimed, 2);
    assert_eq!(report.published, 2);
    assert_eq!(report.retries, 0);
    assert_eq!(report.stale_claims, 0);
    Ok(())
}

#[tokio::test]
async fn concurrent_claimers_skip_locked_rows_and_same_owner_aba_is_generation_fenced() -> TestResult
{
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = insert_owner_fixture(&pool).await?;
    let outbox_id = insert_outbox(&pool, fixture.chatroom_id).await?;
    let repository = Arc::new(PostgresRealtimeRepository::new(pool.clone()));
    let first = repository.claim(claim_request("shared-owner", 2_000));
    let second = repository.claim(claim_request("other-owner", 2_000));
    let (first, second) = tokio::join!(first, second);
    let mut claims = first?;
    claims.extend(second?);
    assert_eq!(claims.len(), 1);
    let stale = claims
        .pop()
        .ok_or("concurrent claimers returned no outbox claim")?;
    assert_eq!(stale.claim_generation, 1);

    sqlx::query(
        "UPDATE outbox_events \
         SET claim_expires_at = clock_timestamp() - INTERVAL '1 millisecond' \
         WHERE id = $1",
    )
    .bind(outbox_id)
    .execute(&pool)
    .await?;
    let fresh = repository
        .claim(claim_request("shared-owner", 2_000))
        .await?
        .pop()
        .ok_or("expired claim was not reclaimed")?;
    assert_eq!(fresh.id, stale.id);
    assert_eq!(fresh.claim_owner, stale.claim_owner);
    assert!(fresh.claim_generation > stale.claim_generation);

    assert!(!repository.mark_published(&stale).await?);
    assert_eq!(
        repository
            .record_failure(
                &stale,
                PublishFailureCode::RedisUnavailable,
                Duration::from_millis(1),
                2,
            )
            .await?,
        FailureDisposition::StaleClaim
    );
    assert!(repository.mark_published(&fresh).await?);
    let (status, generation) = sqlx::query_as::<_, (String, i64)>(
        "SELECT status, claim_generation FROM outbox_events WHERE id = $1",
    )
    .bind(outbox_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(status, "published");
    assert_eq!(generation, fresh.claim_generation);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn failure_state_persists_retry_metadata_then_dead_letters_without_deleting_event()
-> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = insert_owner_fixture(&pool).await?;
    let outbox_id = insert_outbox(&pool, fixture.chatroom_id).await?;
    let repository = PostgresRealtimeRepository::new(pool.clone());

    let first = repository
        .claim(claim_request("retry-worker", 500))
        .await?
        .pop()
        .ok_or("pending event was not claimed")?;
    assert_eq!(
        repository
            .record_failure(
                &first,
                PublishFailureCode::RedisUnavailable,
                Duration::from_millis(1),
                2,
            )
            .await?,
        FailureDisposition::RetryScheduled
    );
    sqlx::query(
        "UPDATE outbox_events \
         SET next_attempt_at = clock_timestamp() - INTERVAL '1 millisecond' \
         WHERE id = $1",
    )
    .bind(outbox_id)
    .execute(&pool)
    .await?;
    let second = repository
        .claim(claim_request("retry-worker", 500))
        .await?
        .pop()
        .ok_or("retry event was not reclaimed")?;
    assert_eq!(second.attempt_count, 1);
    assert_eq!(
        repository
            .record_failure(
                &second,
                PublishFailureCode::PublishTimeout,
                Duration::from_millis(1),
                2,
            )
            .await?,
        FailureDisposition::DeadLettered
    );

    let row = sqlx::query_as::<_, (String, i32, Option<String>, bool, i64)>(
        "SELECT o.status, o.attempt_count, o.last_error_code, \
                o.dead_lettered_at IS NOT NULL, COUNT(e.id) \
         FROM outbox_events o \
         JOIN conversation_events e ON e.id = o.conversation_event_id \
         WHERE o.id = $1 \
         GROUP BY o.id",
    )
    .bind(outbox_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        row,
        (
            "dead_letter".to_owned(),
            2,
            Some("publish_timeout".to_owned()),
            true,
            1
        )
    );

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn subscription_authorization_collapses_every_denied_membership_case() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let owner = insert_owner_fixture(&pool).await?;
    let outsider = insert_owner_fixture(&pool).await?;
    let repository = PostgresRealtimeRepository::new(pool.clone());

    assert!(
        repository
            .is_authorized(owner.user_id, owner.chatroom_id)
            .await?
    );
    for (user_id, conversation_id) in [
        (outsider.user_id, owner.chatroom_id),
        (owner.user_id, outsider.chatroom_id),
        (owner.user_id, Uuid::new_v4()),
    ] {
        assert!(!repository.is_authorized(user_id, conversation_id).await?);
    }

    sqlx::query("UPDATE groups SET deleted_at = clock_timestamp() WHERE id = $1")
        .bind(owner.group_id)
        .execute(&pool)
        .await?;
    assert!(
        !repository
            .is_authorized(owner.user_id, owner.chatroom_id)
            .await?
    );

    pool.close().await;
    database.dispose().await
}

fn claim_request(owner: &str, lease_milliseconds: u64) -> OutboxClaimRequest {
    OutboxClaimRequest {
        claim_owner: owner.to_owned(),
        batch_size: 1,
        lease_duration: Duration::from_millis(lease_milliseconds),
    }
}

async fn insert_outbox(pool: &PgPool, conversation_id: Uuid) -> TestResult<Uuid> {
    let event_id = Uuid::new_v4();
    let outbox_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO conversation_events \
         (id, conversation_id, event_type, event_version, payload) \
         VALUES ($1, $2, 'message.created', 1, $3)",
    )
    .bind(event_id)
    .bind(conversation_id)
    .bind(json!({"fixture": "task-4b"}))
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO outbox_events \
         (id, intent_type, event_type, event_version, aggregate_type, aggregate_id, \
          conversation_event_id, payload) \
         VALUES ($1, 'conversation', 'message.created', 1, 'conversation', $2, $3, $4)",
    )
    .bind(outbox_id)
    .bind(conversation_id)
    .bind(event_id)
    .bind(json!({
        "version": 1,
        "type": "message.created",
        "event_id": event_id,
        "conversation_id": conversation_id,
        "cursor": "1",
        "occurred_at": "1970-01-01T00:00:00Z",
        "data": {
            "id": Uuid::new_v4(),
            "chatroom_id": conversation_id,
            "sender_id": Uuid::new_v4(),
            "client_msg_id": Uuid::new_v4(),
            "body": "task-4b",
            "type": "user",
            "created_at": "1970-01-01T00:00:00Z",
            "media": []
        }
    }))
    .execute(pool)
    .await?;
    Ok(outbox_id)
}

fn worker_config() -> OutboxWorkerConfig {
    OutboxWorkerConfig {
        claim_owner: "task-4b-config".to_owned(),
        batch_size: 1,
        lease_duration: Duration::from_secs(1),
        publish_timeout: Duration::from_millis(200),
        lease_safety_margin: Duration::from_millis(100),
        retry_delay: Duration::from_millis(10),
        poll_interval: Duration::from_millis(10),
        max_attempts: 3,
    }
}

struct NoopRepository;

impl OutboxRepository for NoopRepository {
    fn claim(&self, _request: OutboxClaimRequest) -> RealtimeFuture<'_, Vec<ClaimedOutboxEvent>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn mark_published<'a>(&'a self, _claim: &'a ClaimedOutboxEvent) -> RealtimeFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    fn record_failure<'a>(
        &'a self,
        _claim: &'a ClaimedOutboxEvent,
        _code: PublishFailureCode,
        _retry_delay: Duration,
        _max_attempts: u32,
    ) -> RealtimeFuture<'a, FailureDisposition> {
        Box::pin(async { Ok(FailureDisposition::StaleClaim) })
    }
}

struct NoopPublisher;

impl RealtimeEventPublisher for NoopPublisher {
    fn publish<'a>(&'a self, _event: &'a ClaimedOutboxEvent) -> RealtimeFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

struct BatchRepository;

impl OutboxRepository for BatchRepository {
    fn claim(&self, request: OutboxClaimRequest) -> RealtimeFuture<'_, Vec<ClaimedOutboxEvent>> {
        Box::pin(async move {
            Ok((0..2)
                .map(|sequence| ClaimedOutboxEvent {
                    id: Uuid::new_v4(),
                    conversation_id: Uuid::new_v4(),
                    event_id: Uuid::new_v4(),
                    payload: json!({"sequence": sequence}),
                    claim_owner: request.claim_owner.clone(),
                    claim_generation: 1,
                    claim_expires_at: time::OffsetDateTime::now_utc() + time::Duration::seconds(1),
                    attempt_count: 0,
                })
                .collect())
        })
    }

    fn mark_published<'a>(&'a self, _claim: &'a ClaimedOutboxEvent) -> RealtimeFuture<'a, bool> {
        Box::pin(async { Ok(true) })
    }

    fn record_failure<'a>(
        &'a self,
        _claim: &'a ClaimedOutboxEvent,
        _code: PublishFailureCode,
        _retry_delay: Duration,
        _max_attempts: u32,
    ) -> RealtimeFuture<'a, FailureDisposition> {
        Box::pin(async { Ok(FailureDisposition::StaleClaim) })
    }
}

struct BarrierPublisher(Barrier);

impl RealtimeEventPublisher for BarrierPublisher {
    fn publish<'a>(&'a self, _event: &'a ClaimedOutboxEvent) -> RealtimeFuture<'a, ()> {
        Box::pin(async move {
            self.0.wait().await;
            Ok(())
        })
    }
}
