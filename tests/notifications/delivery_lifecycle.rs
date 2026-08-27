use std::{sync::Arc, time::Duration};

use jamye_server::{
    adapters::postgres::push::PostgresPushRepository,
    ports::push::{
        PushDeliveryClaimRequest, PushDeliveryFailureCode, PushDeliveryFailureDisposition,
        PushDeliveryRepository, PushRepositoryError,
    },
};
use sqlx::PgPool;

use crate::{
    TestResult, postgres_support::TestDatabase, send_authorization::helpers::SendTopology,
};

#[tokio::test]
async fn concurrent_claimers_skip_locked_rows_and_same_owner_aba_is_generation_fenced() -> TestResult
{
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let topology = pending_topology(&pool).await?;
    let repository = Arc::new(PostgresPushRepository::new(pool.clone()));

    let first = repository.claim_deliveries(claim_request("shared-owner", 2_000));
    let second = repository.claim_deliveries(claim_request("other-owner", 2_000));
    let (first, second) = tokio::join!(first, second);
    let mut claims = first?;
    claims.extend(second?);
    assert_eq!(claims.len(), 1);
    let stale = claims
        .pop()
        .ok_or("concurrent push claimers returned no claim")?;
    assert_eq!(stale.claim.claim_generation, 1);

    expire_claim(&pool, topology.occurrence_id).await?;
    let fresh = repository
        .claim_deliveries(claim_request(&stale.claim.claim_owner, 2_000))
        .await?
        .pop()
        .ok_or("expired push occurrence was not reclaimed")?;
    assert_eq!(fresh.claim.occurrence_id, stale.claim.occurrence_id);
    assert_eq!(fresh.claim.claim_owner, stale.claim.claim_owner);
    assert!(fresh.claim.claim_generation > stale.claim.claim_generation);

    assert!(!repository.mark_delivery_succeeded(&stale).await?);
    assert_eq!(
        repository
            .record_delivery_failure(
                &stale,
                PushDeliveryFailureCode::ExpoUnavailable,
                Duration::from_millis(1),
                2,
            )
            .await?,
        PushDeliveryFailureDisposition::StaleClaim
    );
    assert!(repository.mark_delivery_succeeded(&fresh).await?);
    assert_delivery_state(
        &pool,
        topology.occurrence_id,
        "succeeded",
        fresh.claim.claim_generation,
    )
    .await?;

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn an_expired_lease_rejects_completion_until_a_new_generation_is_claimed() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let topology = pending_topology(&pool).await?;
    let repository = PostgresPushRepository::new(pool.clone());
    let expired = repository
        .claim_deliveries(claim_request("lease-owner", 500))
        .await?
        .pop()
        .ok_or("pending push occurrence was not claimed")?;
    expire_claim(&pool, topology.occurrence_id).await?;

    assert!(!repository.mark_delivery_succeeded(&expired).await?);
    assert_eq!(
        repository
            .record_delivery_failure(
                &expired,
                PushDeliveryFailureCode::ExpoTimeout,
                Duration::from_millis(1),
                2,
            )
            .await?,
        PushDeliveryFailureDisposition::StaleClaim
    );
    assert_delivery_state(&pool, topology.occurrence_id, "claimed", 1).await?;

    let reclaimed = repository
        .claim_deliveries(claim_request("lease-owner", 500))
        .await?
        .pop()
        .ok_or("expired push occurrence was not reclaimed")?;
    assert_eq!(reclaimed.claim.claim_generation, 2);
    assert!(repository.mark_delivery_succeeded(&reclaimed).await?);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn retry_metadata_then_dead_letter_preserves_the_single_canonical_occurrence() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let topology = pending_topology(&pool).await?;
    let repository = PostgresPushRepository::new(pool.clone());
    let first = repository
        .claim_deliveries(claim_request("retry-owner", 500))
        .await?
        .pop()
        .ok_or("pending push occurrence was not claimed")?;

    assert_eq!(
        repository
            .record_delivery_failure(
                &first,
                PushDeliveryFailureCode::ExpoUnavailable,
                Duration::from_millis(1),
                2,
            )
            .await?,
        PushDeliveryFailureDisposition::RetryScheduled
    );
    make_retry_due(&pool, topology.occurrence_id).await?;
    let second = repository
        .claim_deliveries(claim_request("retry-owner", 500))
        .await?
        .pop()
        .ok_or("retryable push occurrence was not reclaimed")?;
    assert_eq!(second.attempt_count, 1);
    assert_eq!(
        repository
            .record_delivery_failure(
                &second,
                PushDeliveryFailureCode::ExpoTimeout,
                Duration::from_millis(1),
                2,
            )
            .await?,
        PushDeliveryFailureDisposition::DeadLettered
    );

    assert_dead_letter(&pool, topology.occurrence_id, 2, "expo_timeout").await?;
    let occurrence_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM push_delivery_intents WHERE notification_id = $1",
    )
    .bind(topology.notification_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(occurrence_count, 1);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn retry_delay_crossing_the_deadline_dead_letters_before_max_attempts() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let topology = pending_topology(&pool).await?;
    let repository = PostgresPushRepository::new(pool.clone());
    sqlx::query(
        "UPDATE push_delivery_intents \
         SET deadline_at = clock_timestamp() + INTERVAL '100 milliseconds' \
         WHERE id = $1",
    )
    .bind(topology.occurrence_id)
    .execute(&pool)
    .await?;
    let claim = repository
        .claim_deliveries(claim_request("deadline-owner", 500))
        .await?
        .pop()
        .ok_or("deadline-bounded push occurrence was not claimed")?;

    assert_eq!(
        repository
            .record_delivery_failure(
                &claim,
                PushDeliveryFailureCode::ExpoUnavailable,
                Duration::from_secs(1),
                10,
            )
            .await?,
        PushDeliveryFailureDisposition::DeadLettered
    );
    assert_dead_letter(&pool, topology.occurrence_id, 1, "expo_unavailable").await?;

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn invalid_claim_requests_are_rejected_without_mutating_the_occurrence() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let topology = pending_topology(&pool).await?;
    let repository = PostgresPushRepository::new(pool.clone());
    for request in [
        claim_request("", 500),
        PushDeliveryClaimRequest {
            claim_owner: "owner".to_owned(),
            batch_size: 0,
            lease_duration: Duration::from_millis(500),
        },
        claim_request("owner\ncontrol", 500),
        claim_request(&"x".repeat(129), 500),
        claim_request("owner", 0),
    ] {
        assert_eq!(
            repository.claim_deliveries(request).await,
            Err(PushRepositoryError::InvalidData)
        );
    }
    assert_delivery_state(&pool, topology.occurrence_id, "pending", 0).await?;

    pool.close().await;
    database.dispose().await
}

async fn pending_topology(pool: &PgPool) -> TestResult<SendTopology> {
    let topology = SendTopology::new(pool).await?;
    sqlx::query(
        "UPDATE push_delivery_intents \
         SET status = 'pending', claim_owner = NULL, claim_generation = 0, \
             lease_expires_at = NULL \
         WHERE id = $1",
    )
    .bind(topology.occurrence_id)
    .execute(pool)
    .await?;
    Ok(topology)
}

async fn expire_claim(pool: &PgPool, occurrence_id: uuid::Uuid) -> TestResult {
    sqlx::query(
        "UPDATE push_delivery_intents \
         SET lease_expires_at = clock_timestamp() - INTERVAL '1 millisecond' \
         WHERE id = $1",
    )
    .bind(occurrence_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn make_retry_due(pool: &PgPool, occurrence_id: uuid::Uuid) -> TestResult {
    sqlx::query(
        "UPDATE push_delivery_intents \
         SET next_attempt_at = clock_timestamp() - INTERVAL '1 millisecond' \
         WHERE id = $1",
    )
    .bind(occurrence_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn assert_delivery_state(
    pool: &PgPool,
    occurrence_id: uuid::Uuid,
    expected_status: &str,
    expected_generation: i64,
) -> TestResult {
    let state = sqlx::query_as::<_, (String, i64)>(
        "SELECT status, claim_generation FROM push_delivery_intents WHERE id = $1",
    )
    .bind(occurrence_id)
    .fetch_one(pool)
    .await?;
    assert_eq!(state, (expected_status.to_owned(), expected_generation));
    Ok(())
}

async fn assert_dead_letter(
    pool: &PgPool,
    occurrence_id: uuid::Uuid,
    expected_attempt_count: i32,
    expected_error_code: &str,
) -> TestResult {
    let state = sqlx::query_as::<_, (String, i32, Option<String>, bool)>(
        "SELECT status, attempt_count, last_error_code, dead_lettered_at IS NOT NULL \
         FROM push_delivery_intents WHERE id = $1",
    )
    .bind(occurrence_id)
    .fetch_one(pool)
    .await?;
    assert_eq!(
        state,
        (
            "dead_letter".to_owned(),
            expected_attempt_count,
            Some(expected_error_code.to_owned()),
            true,
        )
    );
    Ok(())
}

fn claim_request(owner: &str, lease_milliseconds: u64) -> PushDeliveryClaimRequest {
    PushDeliveryClaimRequest {
        claim_owner: owner.to_owned(),
        batch_size: 1,
        lease_duration: Duration::from_millis(lease_milliseconds),
    }
}
