use std::{
    collections::VecDeque,
    fmt::Debug,
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Request, Response, StatusCode},
    routing::any,
};
use jamye_server::{
    adapters::{
        object_storage::account_deletion::{
            S3AccountObjectDeletionConfigError, S3AccountObjectDeletionCredentials,
            S3AccountObjectDeletionProvider,
        },
        postgres::account_deletion::PostgresAccountDeletionRepository,
    },
    application::account_deletion::cleanup::{
        AccountObjectDeletionWorker, AccountObjectDeletionWorkerConfig,
        AccountObjectDeletionWorkerDependencies, AccountObjectDeletionWorkerReport,
    },
    config::{
        AppEnvironment,
        object_storage::{ObjectStorageConfig, ObjectStorageConfigInput},
    },
    ports::{
        account_deletion::{
            AccountObjectDeletionClaim, AccountObjectDeletionClaimRequest,
            AccountObjectDeletionFailureCode, AccountObjectDeletionFailureDisposition,
            AccountObjectDeletionProvider, AccountObjectDeletionProviderFuture,
            AccountObjectDeletionRepository, AccountObjectDeletionRepositoryFuture,
        },
        object_storage::ObjectStorageProviderError,
    },
};
use sqlx::{Connection, PgPool};
use time::OffsetDateTime;
use tokio::{
    net::TcpListener,
    sync::{Barrier, oneshot},
    task::JoinHandle,
};
use uuid::Uuid;

use crate::{
    TestResult,
    postgres_support::TestDatabase,
    support::{require, require_eq},
};

const MEDIA_ACCESS_KEY_ID: &str = "task-8-media-access";
const MEDIA_SECRET_ACCESS_KEY: &str = "task-8-media-secret-must-not-appear";
const CLEANUP_ACCESS_KEY_ID: &str = "task-11-s3-access";
const SECRET_ACCESS_KEY: &str = "task-11-s3-secret-must-not-appear";

#[tokio::test]
async fn worker_rejects_non_strict_or_invalid_config_without_claim_or_provider_calls() -> TestResult
{
    let repository = Arc::new(RecordingRepository::default());
    let provider = Arc::new(RecordingProvider::default());
    let dependencies = dependencies(repository.clone(), provider.clone());

    let accepted_invalid = invalid_configurations()
        .into_iter()
        .filter_map(|(name, configuration)| {
            AccountObjectDeletionWorker::new(dependencies.clone(), configuration)
                .is_ok()
                .then_some(name)
        })
        .collect::<Vec<_>>();
    assert_eq!(repository.calls.load(Ordering::SeqCst), 0);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);

    let mut internal_space_owner = worker_config();
    internal_space_owner.claim_owner = "cleanup worker".to_owned();
    require(
        AccountObjectDeletionWorker::new(dependencies.clone(), internal_space_owner).is_ok(),
        "cleanup worker rejected a legitimate internal-space owner",
    )?;
    require_eq(
        accepted_invalid,
        Vec::<&'static str>::new(),
        "cleanup worker accepted invalid configurations",
    )?;

    let mut boundary = worker_config();
    boundary.delete_timeout = Duration::from_millis(40);
    boundary.lease_safety_margin = Duration::from_millis(10);
    boundary.lease_duration = Duration::from_millis(51);
    require(
        AccountObjectDeletionWorker::new(dependencies.clone(), boundary).is_ok(),
        "lease one millisecond above the strict cleanup budget was rejected",
    )?;

    let worker = AccountObjectDeletionWorker::new(dependencies, worker_config())?;
    let outcome = tokio::time::timeout(Duration::from_millis(500), worker.run_once()).await?;
    require_eq(
        outcome,
        Ok(AccountObjectDeletionWorkerReport {
            claimed: 1,
            succeeded: 1,
            ..AccountObjectDeletionWorkerReport::default()
        }),
        "valid cleanup worker did not claim, delete, and complete one object",
    )?;
    require_eq(
        repository.calls.load(Ordering::SeqCst),
        1,
        "valid worker did not issue exactly one claim",
    )?;
    require_eq(
        provider.calls.load(Ordering::SeqCst),
        1,
        "valid worker did not issue exactly one delete",
    )
}

#[tokio::test]
async fn two_hanging_deletes_start_together_and_timeout_to_retry_without_false_success()
-> TestResult {
    let repository = Arc::new(ClaimingRepository::with_claims(two_claims()));
    let provider = Arc::new(HangingProvider::new());
    let mut config = worker_config();
    config.batch_size = 2;
    config.delete_timeout = Duration::from_millis(40);
    config.lease_safety_margin = Duration::from_millis(10);
    config.lease_duration = Duration::from_millis(250);
    let worker = AccountObjectDeletionWorker::new(
        dependencies(repository.clone(), provider.clone()),
        config,
    )?;

    let outcome = tokio::time::timeout(Duration::from_millis(500), worker.run_once()).await?;
    require_eq(
        provider.starts.load(Ordering::SeqCst),
        2,
        "both hanging deletes did not enter the start barrier concurrently",
    )?;
    require_eq(
        repository.retry_calls.load(Ordering::SeqCst),
        2,
        "timed-out deletes did not each record retryable state",
    )?;
    require_eq(
        repository.success_calls.load(Ordering::SeqCst),
        0,
        "timed-out deletes falsely completed cleanup",
    )?;
    require_eq(
        outcome,
        Ok(AccountObjectDeletionWorkerReport {
            claimed: 2,
            retries: 2,
            ..AccountObjectDeletionWorkerReport::default()
        }),
        "hanging deletes did not become two retryable outcomes",
    )
}

#[tokio::test]
async fn postgres_claiming_uses_skip_locked_expiry_reclaim_and_generation_fences_late_results()
-> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let locked_id = fixture_id(301);
        let unlocked_first_id = fixture_id(302);
        let unlocked_second_id = fixture_id(303);
        for (id, key) in [
            (locked_id, "account-delete/001-locked"),
            (unlocked_first_id, "account-delete/002-unlocked"),
            (unlocked_second_id, "account-delete/003-unlocked"),
        ] {
            seed_intent(&pool, id, key).await?;
        }
        let repository = PostgresAccountDeletionRepository::new(pool.clone());
        let mut lock_connection = database.connection().await?;
        let mut lock_transaction = lock_connection.begin().await?;
        sqlx::query("SELECT id FROM account_object_deletion_intents WHERE id = $1 FOR UPDATE")
            .bind(locked_id)
            .execute(&mut *lock_transaction)
            .await?;

        let unlocked = claims_or_error(
            repository
                .claim_object_deletions(claim_request("skip-locked-owner", 2, 2_000))
                .await,
            "SKIP LOCKED claimer returned a repository error instead of unlocked rows",
        )?;
        require_eq(
            unlocked
                .iter()
                .map(|claim| claim.intent_id)
                .collect::<Vec<_>>(),
            vec![unlocked_first_id, unlocked_second_id],
            "claimer did not skip the locked first due row in deterministic order",
        )?;
        lock_transaction.rollback().await?;
        drop(lock_connection);

        let first = one_claim(
            repository
                .claim_object_deletions(claim_request("same-owner", 1, 2_000))
                .await,
            "unlocked original intent was not claimed for ABA coverage",
        )?;
        require_eq(
            first.intent_id,
            locked_id,
            "unexpected intent selected after lock release",
        )?;
        expire_claim(&pool, first.intent_id).await?;
        let replacement = one_claim(
            repository
                .claim_object_deletions(claim_request("same-owner", 1, 2_000))
                .await,
            "expired same-owner intent was not reclaimed",
        )?;
        require_eq(
            replacement.intent_id,
            first.intent_id,
            "same-owner reclaim selected a different intent",
        )?;
        require(
            replacement.claim_generation > first.claim_generation,
            "same-owner reclaim did not advance generation",
        )?;
        require_eq(
            repository.mark_object_deleted(&first).await,
            Ok(false),
            "stale successful delete mutated a reclaimed intent",
        )?;
        require_eq(
            repository
                .record_object_deletion_failure(
                    &first,
                    AccountObjectDeletionFailureCode::Unavailable,
                    Duration::from_millis(10),
                    3,
                )
                .await,
            Ok(AccountObjectDeletionFailureDisposition::StaleClaim),
            "stale failed delete mutated a reclaimed intent",
        )?;
        require_eq(
            repository.mark_object_deleted(&replacement).await,
            Ok(true),
            "current generation was not allowed to complete",
        )
    }
    .await;
    finish_database_test(database, pool, result).await
}

#[tokio::test]
async fn retry_metadata_attempts_and_deadline_terminalize_one_intent() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let retry_id = fixture_id(402);
        seed_intent(&pool, retry_id, "account-delete/retry").await?;
        let deadline_id = fixture_id(401);
        seed_intent(&pool, deadline_id, "account-delete/deadline").await?;
        sqlx::query(
            "UPDATE account_object_deletion_intents \
             SET deadline_at = clock_timestamp() + INTERVAL '100 milliseconds' \
             WHERE id = $1",
        )
        .bind(deadline_id)
        .execute(&pool)
        .await?;
        order_retry_before_deadline(&pool, retry_id, deadline_id).await?;
        let repository = PostgresAccountDeletionRepository::new(pool.clone());

        let first = one_claim(
            repository
                .claim_object_deletions(claim_request("retry-worker", 1, 500))
                .await,
            "retry intent was not claimed",
        )?;
        require_eq(
            first.intent_id,
            retry_id,
            "first due cleanup intent changed",
        )?;
        require_eq(
            repository
                .record_object_deletion_failure(
                    &first,
                    AccountObjectDeletionFailureCode::Unavailable,
                    Duration::from_secs(1),
                    2,
                )
                .await,
            Ok(AccountObjectDeletionFailureDisposition::RetryScheduled),
            "first cleanup failure was not made retryable",
        )?;
        require_eq(
            cleanup_snapshot(&pool, retry_id).await?,
            CleanupSnapshot::retryable(1, "unavailable"),
            "retry metadata was not durably recorded",
        )?;
        order_retry_before_deadline(&pool, retry_id, deadline_id).await?;
        let second = one_claim(
            repository
                .claim_object_deletions(claim_request("retry-worker", 1, 500))
                .await,
            "retryable cleanup intent was not reclaimed",
        )?;
        require_eq(
            second.intent_id,
            retry_id,
            "reclaim selected a different intent",
        )?;
        require_eq(
            second.attempt_count,
            1,
            "retry attempt count was not preserved",
        )?;
        require_eq(
            repository
                .record_object_deletion_failure(
                    &second,
                    AccountObjectDeletionFailureCode::Timeout,
                    Duration::from_millis(10),
                    2,
                )
                .await,
            Ok(AccountObjectDeletionFailureDisposition::DeadLettered),
            "max-attempt cleanup failure was not dead-lettered",
        )?;
        require_eq(
            cleanup_snapshot(&pool, retry_id).await?,
            CleanupSnapshot::dead_letter(2, "timeout", 2),
            "dead-letter cleanup metadata is not canonical",
        )?;

        let deadline = one_claim(
            repository
                .claim_object_deletions(claim_request("deadline-worker", 1, 500))
                .await,
            "deadline cleanup intent was not claimed",
        )?;
        require_eq(
            deadline.intent_id,
            deadline_id,
            "deadline fixture order changed",
        )?;
        require_eq(
            repository
                .record_object_deletion_failure(
                    &deadline,
                    AccountObjectDeletionFailureCode::Unavailable,
                    Duration::from_secs(1),
                    10,
                )
                .await,
            Ok(AccountObjectDeletionFailureDisposition::DeadLettered),
            "retry delay crossing a deadline did not dead-letter cleanup",
        )?;
        require_eq(
            cleanup_snapshot(&pool, deadline_id).await?,
            CleanupSnapshot::dead_letter(1, "unavailable", 1),
            "deadline cleanup metadata is not canonical",
        )?;
        require_eq(
            intent_count(&pool).await?,
            2,
            "retry/dead-letter transitions created duplicate cleanup rows",
        )
    }
    .await;
    finish_database_test(database, pool, result).await
}

#[tokio::test]
async fn terminal_cleanup_states_are_monotonic_unreclaimable_and_ignore_late_results() -> TestResult
{
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let terminal_ids = [fixture_id(501), fixture_id(502), fixture_id(503)];
        seed_terminal_intent(
            &pool,
            terminal_ids[0],
            "account-delete/succeeded",
            "succeeded",
        )
        .await?;
        seed_terminal_intent(&pool, terminal_ids[1], "account-delete/failed", "failed").await?;
        seed_terminal_intent(
            &pool,
            terminal_ids[2],
            "account-delete/dead-letter",
            "dead_letter",
        )
        .await?;
        let repository = PostgresAccountDeletionRepository::new(pool.clone());
        require_eq(
            repository
                .claim_object_deletions(claim_request("terminal-worker", 3, 500))
                .await,
            Ok(Vec::new()),
            "terminal cleanup rows were reclaimable",
        )?;
        for intent_id in terminal_ids {
            let before = durable_snapshot(&pool, intent_id).await?;
            let fabricated = AccountObjectDeletionClaim::new(
                intent_id,
                "account-delete/redacted".to_owned(),
                "late-worker".to_owned(),
                1,
                OffsetDateTime::now_utc() + time::Duration::seconds(1),
                0,
            );
            require_eq(
                repository.mark_object_deleted(&fabricated).await,
                Ok(false),
                "late success mutated a terminal cleanup row",
            )?;
            require_eq(
                repository
                    .record_object_deletion_failure(
                        &fabricated,
                        AccountObjectDeletionFailureCode::UnexpectedResponse,
                        Duration::from_millis(1),
                        2,
                    )
                    .await,
                Ok(AccountObjectDeletionFailureDisposition::StaleClaim),
                "late failure mutated a terminal cleanup row",
            )?;
            require_eq(
                durable_snapshot(&pool, intent_id).await?,
                before,
                "late terminal result changed durable cleanup state",
            )?;
        }
        Ok(())
    }
    .await;
    finish_database_test(database, pool, result).await
}

#[tokio::test]
async fn s3_cleanup_rejects_media_credentials_and_signs_with_its_dedicated_identity() -> TestResult
{
    let internal = ScriptedS3::start([StatusCode::NO_CONTENT]).await?;
    let public = ScriptedS3::start([]).await?;
    let config = object_storage_config(internal.endpoint(), public.endpoint())?;
    let reused =
        S3AccountObjectDeletionCredentials::new(MEDIA_ACCESS_KEY_ID, MEDIA_SECRET_ACCESS_KEY)?;
    require(
        matches!(
            S3AccountObjectDeletionProvider::new(&config, &reused),
            Err(S3AccountObjectDeletionConfigError::ReusesMediaCredentials)
        ),
        "S3 cleanup accepted the Task-8 media identity",
    )?;

    let cleanup_credentials = cleanup_credentials()?;
    require(
        !format!("{cleanup_credentials:?}").contains(SECRET_ACCESS_KEY),
        "S3 cleanup credentials Debug exposed a secret",
    )?;
    let storage = S3AccountObjectDeletionProvider::new(&config, &cleanup_credentials)?;
    let outcome = tokio::time::timeout(
        Duration::from_millis(500),
        storage.delete_object("account-delete/dedicated-identity"),
    )
    .await?;
    require_eq(outcome, Ok(()), "dedicated cleanup identity DELETE failed")?;

    let internal_requests = internal.finish().await?;
    let public_requests = public.finish().await?;
    require_eq(
        internal_requests.len(),
        1,
        "cleanup identity sent an unexpected request count",
    )?;
    require_eq(
        public_requests.len(),
        0,
        "cleanup identity used the public S3 origin",
    )?;
    let request = &internal_requests[0];
    require(
        request.uses_cleanup_identity,
        "DELETE was not signed by the cleanup identity",
    )?;
    require(
        !request.uses_media_identity,
        "DELETE was signed by the Task-8 media identity",
    )
}

#[tokio::test]
async fn s3_delete_is_signed_internal_path_style_idempotent_and_secret_safe() -> TestResult {
    let internal = ScriptedS3::start([
        StatusCode::NO_CONTENT,
        StatusCode::NO_CONTENT,
        StatusCode::FORBIDDEN,
        StatusCode::NOT_FOUND,
        StatusCode::SERVICE_UNAVAILABLE,
    ])
    .await?;
    let public = ScriptedS3::start([]).await?;
    let config = object_storage_config(internal.endpoint(), public.endpoint())?;
    let cleanup_credentials = cleanup_credentials()?;
    let storage = S3AccountObjectDeletionProvider::new(&config, &cleanup_credentials)?;
    require(
        !format!("{storage:?}").contains(SECRET_ACCESS_KEY),
        "S3 cleanup provider Debug exposed a secret",
    )?;
    let object_keys = [
        "account-delete/one",
        "account-delete/one",
        "account-delete/denied",
        "account-delete/unexpected-not-found",
        "account-delete/unavailable",
    ];
    let mut outcomes = Vec::new();
    for object_key in object_keys {
        outcomes.push(
            tokio::time::timeout(
                Duration::from_millis(500),
                storage.delete_object(object_key),
            )
            .await?,
        );
    }
    let internal_requests = internal.finish().await?;
    let public_requests = public.finish().await?;

    require_eq(
        outcomes[0],
        Ok(()),
        "first idempotent DELETE did not accept 204",
    )?;
    require_eq(
        outcomes[1],
        Ok(()),
        "second idempotent DELETE did not accept 204",
    )?;
    require_eq(
        outcomes[2],
        Err(ObjectStorageProviderError::AccessDenied),
        "403 DELETE was not classified as access denied",
    )?;
    require_eq(
        outcomes[3],
        Err(ObjectStorageProviderError::UnexpectedResponse),
        "ambiguous 404 DELETE was incorrectly treated as object absence",
    )?;
    require_eq(
        outcomes[4],
        Err(ObjectStorageProviderError::Unavailable),
        "503 DELETE was not classified as retryable unavailable",
    )?;
    require_eq(
        public_requests.len(),
        0,
        "public S3 origin received internal DELETE",
    )?;
    require_eq(
        internal_requests.len(),
        object_keys.len(),
        "internal S3 origin did not receive every DELETE",
    )?;
    for (request, object_key) in internal_requests.iter().zip(object_keys) {
        require_eq(
            request.method.as_str(),
            "DELETE",
            "S3 cleanup used the wrong method",
        )?;
        let expected_path = format!("/jamye-private-bucket/{object_key}");
        require_eq(
            request.path.as_str(),
            expected_path.as_str(),
            "S3 cleanup did not use signed internal path-style key routing",
        )?;
        require(request.signed, "S3 cleanup DELETE was not SigV4 signed")?;
        require(
            !request.secret_exposed,
            "S3 cleanup DELETE exposed the secret",
        )?;
    }
    Ok(())
}

#[tokio::test]
async fn postgres_and_s3_503_retry_then_204_succeeds_once_and_third_poll_is_empty() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let intent_id = fixture_id(701);
        seed_intent(&pool, intent_id, "account-delete/s3-retry").await?;
        let internal =
            ScriptedS3::start([StatusCode::SERVICE_UNAVAILABLE, StatusCode::NO_CONTENT]).await?;
        let public = ScriptedS3::start([]).await?;
        let config = object_storage_config(internal.endpoint(), public.endpoint())?;
        let cleanup_credentials = cleanup_credentials()?;
        let provider = Arc::new(S3AccountObjectDeletionProvider::new(
            &config,
            &cleanup_credentials,
        )?);
        let repository = Arc::new(PostgresAccountDeletionRepository::new(pool.clone()));
        let worker =
            AccountObjectDeletionWorker::new(dependencies(repository, provider), worker_config())?;

        let first = tokio::time::timeout(Duration::from_millis(500), worker.run_once()).await?;
        require_eq(
            first,
            Ok(AccountObjectDeletionWorkerReport {
                claimed: 1,
                retries: 1,
                ..AccountObjectDeletionWorkerReport::default()
            }),
            "503 did not become a retryable cleanup outcome",
        )?;
        require_eq(
            cleanup_snapshot(&pool, intent_id).await?,
            CleanupSnapshot::retryable(1, "unavailable"),
            "503 retry did not persist durable cleanup metadata",
        )?;
        make_retry_due(&pool, intent_id).await?;
        let second = tokio::time::timeout(Duration::from_millis(500), worker.run_once()).await?;
        require_eq(
            second,
            Ok(AccountObjectDeletionWorkerReport {
                claimed: 1,
                succeeded: 1,
                ..AccountObjectDeletionWorkerReport::default()
            }),
            "204 did not complete the retried cleanup intent",
        )?;
        let third = tokio::time::timeout(Duration::from_millis(500), worker.run_once()).await?;
        require_eq(
            third,
            Ok(AccountObjectDeletionWorkerReport::default()),
            "succeeded cleanup intent was claimed again",
        )?;
        require_eq(
            cleanup_snapshot(&pool, intent_id).await?,
            CleanupSnapshot::succeeded(1, 2),
            "S3 retry convergence did not leave one succeeded generation-two row",
        )?;
        let internal_requests = internal.finish().await?;
        let public_requests = public.finish().await?;
        require_eq(
            internal_requests.len(),
            2,
            "S3 retry did not issue exactly two DELETEs",
        )?;
        require_eq(public_requests.len(), 0, "S3 retry used the public origin")
    }
    .await;
    finish_database_test(database, pool, result).await
}

fn worker_config() -> AccountObjectDeletionWorkerConfig {
    AccountObjectDeletionWorkerConfig {
        claim_owner: "task-11-cleanup-worker".to_owned(),
        batch_size: 2,
        lease_duration: Duration::from_secs(3),
        delete_timeout: Duration::from_secs(1),
        lease_safety_margin: Duration::from_millis(200),
        retry_delay: Duration::from_millis(25),
        poll_interval: Duration::from_millis(25),
        max_attempts: 3,
    }
}

fn invalid_configurations() -> Vec<(&'static str, AccountObjectDeletionWorkerConfig)> {
    let mut equal_budget = worker_config();
    equal_budget.lease_duration = equal_budget.delete_timeout + equal_budget.lease_safety_margin;
    let mut below_budget = worker_config();
    below_budget.lease_duration = below_budget.delete_timeout;
    let mut zero_lease = worker_config();
    zero_lease.lease_duration = Duration::ZERO;
    let mut zero_timeout = worker_config();
    zero_timeout.delete_timeout = Duration::ZERO;
    let mut zero_margin = worker_config();
    zero_margin.lease_safety_margin = Duration::ZERO;
    let mut zero_retry = worker_config();
    zero_retry.retry_delay = Duration::ZERO;
    let mut zero_poll = worker_config();
    zero_poll.poll_interval = Duration::ZERO;
    let mut zero_batch = worker_config();
    zero_batch.batch_size = 0;
    let mut zero_attempts = worker_config();
    zero_attempts.max_attempts = 0;
    let mut empty_owner = worker_config();
    empty_owner.claim_owner.clear();
    let mut whitespace_owner = worker_config();
    whitespace_owner.claim_owner = "   ".to_owned();
    let mut padded_owner = worker_config();
    padded_owner.claim_owner = " cleanup-worker ".to_owned();
    let mut control_owner = worker_config();
    control_owner.claim_owner = "cleanup\nowner".to_owned();
    let mut long_owner = worker_config();
    long_owner.claim_owner = "x".repeat(129);
    let mut overflow = worker_config();
    overflow.delete_timeout = Duration::MAX;
    overflow.lease_safety_margin = Duration::from_nanos(1);
    vec![
        ("equal_budget", equal_budget),
        ("below_budget", below_budget),
        ("zero_lease", zero_lease),
        ("zero_timeout", zero_timeout),
        ("zero_margin", zero_margin),
        ("zero_retry", zero_retry),
        ("zero_poll", zero_poll),
        ("zero_batch", zero_batch),
        ("zero_attempts", zero_attempts),
        ("empty_owner", empty_owner),
        ("whitespace_owner", whitespace_owner),
        ("padded_owner", padded_owner),
        ("control_owner", control_owner),
        ("long_owner", long_owner),
        ("overflow", overflow),
    ]
}

fn dependencies(
    repository: Arc<dyn AccountObjectDeletionRepository>,
    provider: Arc<dyn AccountObjectDeletionProvider>,
) -> AccountObjectDeletionWorkerDependencies {
    AccountObjectDeletionWorkerDependencies {
        repository,
        provider,
    }
}

fn claim_request(
    owner: &str,
    batch_size: u32,
    lease_milliseconds: u64,
) -> AccountObjectDeletionClaimRequest {
    AccountObjectDeletionClaimRequest {
        claim_owner: owner.to_owned(),
        batch_size,
        lease_duration: Duration::from_millis(lease_milliseconds),
    }
}

fn claim(number: u128, key: &str) -> AccountObjectDeletionClaim {
    AccountObjectDeletionClaim::new(
        fixture_id(number),
        key.to_owned(),
        "task-11-cleanup-worker".to_owned(),
        1,
        OffsetDateTime::now_utc() + time::Duration::seconds(1),
        0,
    )
}

fn two_claims() -> Vec<AccountObjectDeletionClaim> {
    vec![
        claim(201, "account-delete/hanging-one"),
        claim(202, "account-delete/hanging-two"),
    ]
}

fn fixture_id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

async fn seed_intent(pool: &PgPool, id: Uuid, object_key: &str) -> TestResult {
    sqlx::query("INSERT INTO account_object_deletion_intents (id, object_key) VALUES ($1, $2)")
        .bind(id)
        .bind(object_key)
        .execute(pool)
        .await?;
    Ok(())
}

async fn seed_terminal_intent(
    pool: &PgPool,
    id: Uuid,
    object_key: &str,
    status: &str,
) -> TestResult {
    match status {
        "succeeded" => {
            sqlx::query(
                "WITH server_clock AS MATERIALIZED (SELECT clock_timestamp() AS now) \
                 INSERT INTO account_object_deletion_intents \
                 (id, object_key, status, succeeded_at, created_at) \
                 SELECT $1, $2, 'succeeded', now, now FROM server_clock",
            )
            .bind(id)
            .bind(object_key)
            .execute(pool)
            .await?;
        }
        "failed" => {
            sqlx::query(
                "WITH server_clock AS MATERIALIZED (SELECT clock_timestamp() AS now) \
                 INSERT INTO account_object_deletion_intents \
                 (id, object_key, status, failed_at, created_at) \
                 SELECT $1, $2, 'failed', now, now FROM server_clock",
            )
            .bind(id)
            .bind(object_key)
            .execute(pool)
            .await?;
        }
        "dead_letter" => {
            sqlx::query(
                "WITH server_clock AS MATERIALIZED (SELECT clock_timestamp() AS now) \
                 INSERT INTO account_object_deletion_intents \
                 (id, object_key, status, dead_lettered_at, created_at) \
                 SELECT $1, $2, 'dead_letter', now, now FROM server_clock",
            )
            .bind(id)
            .bind(object_key)
            .execute(pool)
            .await?;
        }
        _ => return Err(io::Error::other("unsupported cleanup terminal fixture").into()),
    }
    Ok(())
}

async fn expire_claim(pool: &PgPool, intent_id: Uuid) -> TestResult {
    sqlx::query(
        "UPDATE account_object_deletion_intents \
         SET lease_expires_at = clock_timestamp() - INTERVAL '1 millisecond' \
         WHERE id = $1",
    )
    .bind(intent_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn make_retry_due(pool: &PgPool, intent_id: Uuid) -> TestResult {
    sqlx::query(
        "UPDATE account_object_deletion_intents \
         SET next_attempt_at = clock_timestamp() - INTERVAL '1 millisecond' \
         WHERE id = $1",
    )
    .bind(intent_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn order_retry_before_deadline(
    pool: &PgPool,
    retry_id: Uuid,
    deadline_id: Uuid,
) -> TestResult {
    let result = sqlx::query(
        "WITH server_clock AS MATERIALIZED (SELECT clock_timestamp() AS now) \
         UPDATE account_object_deletion_intents AS intent \
         SET next_attempt_at = CASE intent.id \
             WHEN $1 THEN (SELECT now FROM server_clock) - INTERVAL '2 milliseconds' \
             WHEN $2 THEN (SELECT now FROM server_clock) - INTERVAL '1 millisecond' \
         END \
         WHERE intent.id IN ($1, $2)",
    )
    .bind(retry_id)
    .bind(deadline_id)
    .execute(pool)
    .await?;
    if result.rows_affected() != 2 {
        return Err(
            io::Error::other("cleanup due-order fixture did not update both intents").into(),
        );
    }
    Ok(())
}

async fn intent_count(pool: &PgPool) -> TestResult<i64> {
    Ok(
        sqlx::query_scalar("SELECT COUNT(*) FROM account_object_deletion_intents")
            .fetch_one(pool)
            .await?,
    )
}

async fn cleanup_snapshot(pool: &PgPool, intent_id: Uuid) -> TestResult<CleanupSnapshot> {
    let row = sqlx::query_as::<_, (String, i32, Option<String>, bool, bool, bool, bool, i64)>(
        "SELECT status, attempt_count, last_error_code, next_attempt_at IS NOT NULL, \
                succeeded_at IS NOT NULL, failed_at IS NOT NULL, dead_lettered_at IS NOT NULL, \
                claim_generation \
         FROM account_object_deletion_intents WHERE id = $1",
    )
    .bind(intent_id)
    .fetch_one(pool)
    .await?;
    Ok(CleanupSnapshot {
        status: row.0,
        attempt_count: row.1,
        last_error_code: row.2,
        retry_scheduled: row.3,
        succeeded: row.4,
        failed: row.5,
        dead_lettered: row.6,
        generation: row.7,
    })
}

async fn durable_snapshot(pool: &PgPool, intent_id: Uuid) -> TestResult<DurableCleanupSnapshot> {
    let snapshot = sqlx::query_scalar::<_, String>(
        "SELECT to_jsonb(intent)::TEXT \
         FROM account_object_deletion_intents AS intent \
         WHERE intent.id = $1",
    )
    .bind(intent_id)
    .fetch_one(pool)
    .await?;
    Ok(DurableCleanupSnapshot(snapshot))
}

async fn finish_database_test(
    database: TestDatabase,
    pool: PgPool,
    result: TestResult,
) -> TestResult {
    pool.close().await;
    let dispose = database.dispose().await;
    result.and(dispose)
}

fn object_storage_config(
    internal_endpoint: &str,
    public_endpoint: &str,
) -> TestResult<ObjectStorageConfig> {
    ObjectStorageConfig::resolve(
        AppEnvironment::Test,
        ObjectStorageConfigInput {
            endpoint: Some(internal_endpoint.to_owned()),
            public_endpoint: Some(public_endpoint.to_owned()),
            region: Some("us-east-1".to_owned()),
            bucket: Some("jamye-private-bucket".to_owned()),
            access_key_id: Some(MEDIA_ACCESS_KEY_ID.to_owned()),
            secret_access_key: Some(MEDIA_SECRET_ACCESS_KEY.to_owned()),
        },
    )?
    .ok_or_else(|| io::Error::other("complete object-storage config resolved absent").into())
}

fn cleanup_credentials() -> TestResult<S3AccountObjectDeletionCredentials> {
    Ok(S3AccountObjectDeletionCredentials::new(
        CLEANUP_ACCESS_KEY_ID,
        SECRET_ACCESS_KEY,
    )?)
}

fn one_claim(
    result: Result<Vec<AccountObjectDeletionClaim>, impl Debug>,
    message: &str,
) -> TestResult<AccountObjectDeletionClaim> {
    let mut claims = result.map_err(|error| io::Error::other(format!("{message}: {error:?}")))?;
    if claims.len() != 1 {
        return Err(io::Error::other(format!(
            "{message}: expected one claim, got {}",
            claims.len()
        ))
        .into());
    }
    claims.pop().ok_or_else(|| io::Error::other(message).into())
}

fn claims_or_error(
    result: Result<Vec<AccountObjectDeletionClaim>, impl Debug>,
    message: &str,
) -> TestResult<Vec<AccountObjectDeletionClaim>> {
    result.map_err(|error| io::Error::other(format!("{message}: {error:?}")).into())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CleanupSnapshot {
    status: String,
    attempt_count: i32,
    last_error_code: Option<String>,
    retry_scheduled: bool,
    succeeded: bool,
    failed: bool,
    dead_lettered: bool,
    generation: i64,
}

#[derive(Clone, Eq, PartialEq)]
struct DurableCleanupSnapshot(String);

impl Debug for DurableCleanupSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("DurableCleanupSnapshot([REDACTED])")
    }
}

impl CleanupSnapshot {
    fn retryable(attempt_count: i32, error_code: &str) -> Self {
        Self {
            status: "retryable".to_owned(),
            attempt_count,
            last_error_code: Some(error_code.to_owned()),
            retry_scheduled: true,
            succeeded: false,
            failed: false,
            dead_lettered: false,
            generation: 1,
        }
    }

    fn dead_letter(attempt_count: i32, error_code: &str, generation: i64) -> Self {
        Self {
            status: "dead_letter".to_owned(),
            attempt_count,
            last_error_code: Some(error_code.to_owned()),
            retry_scheduled: false,
            succeeded: false,
            failed: false,
            dead_lettered: true,
            generation,
        }
    }

    fn succeeded(attempt_count: i32, generation: i64) -> Self {
        Self {
            status: "succeeded".to_owned(),
            attempt_count,
            last_error_code: None,
            retry_scheduled: false,
            succeeded: true,
            failed: false,
            dead_lettered: false,
            generation,
        }
    }
}

#[derive(Default)]
struct RecordingRepository {
    calls: AtomicUsize,
}

impl AccountObjectDeletionRepository for RecordingRepository {
    fn claim_object_deletions(
        &self,
        _request: AccountObjectDeletionClaimRequest,
    ) -> AccountObjectDeletionRepositoryFuture<'_, Vec<AccountObjectDeletionClaim>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(vec![claim(101, "account-delete/recorded")]) })
    }

    fn mark_object_deleted<'a>(
        &'a self,
        _claim: &'a AccountObjectDeletionClaim,
    ) -> AccountObjectDeletionRepositoryFuture<'a, bool> {
        Box::pin(async { Ok(true) })
    }

    fn record_object_deletion_failure<'a>(
        &'a self,
        _claim: &'a AccountObjectDeletionClaim,
        _code: AccountObjectDeletionFailureCode,
        _retry_delay: Duration,
        _max_attempts: u32,
    ) -> AccountObjectDeletionRepositoryFuture<'a, AccountObjectDeletionFailureDisposition> {
        Box::pin(async { Ok(AccountObjectDeletionFailureDisposition::RetryScheduled) })
    }
}

#[derive(Default)]
struct RecordingProvider {
    calls: AtomicUsize,
}

impl AccountObjectDeletionProvider for RecordingProvider {
    fn delete_object<'a>(
        &'a self,
        _object_key: &'a str,
    ) -> AccountObjectDeletionProviderFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }
}

struct ClaimingRepository {
    claims: Vec<AccountObjectDeletionClaim>,
    retry_calls: AtomicUsize,
    success_calls: AtomicUsize,
}

impl ClaimingRepository {
    fn with_claims(claims: Vec<AccountObjectDeletionClaim>) -> Self {
        Self {
            claims,
            retry_calls: AtomicUsize::default(),
            success_calls: AtomicUsize::default(),
        }
    }
}

impl AccountObjectDeletionRepository for ClaimingRepository {
    fn claim_object_deletions(
        &self,
        _request: AccountObjectDeletionClaimRequest,
    ) -> AccountObjectDeletionRepositoryFuture<'_, Vec<AccountObjectDeletionClaim>> {
        Box::pin(async move { Ok(self.claims.clone()) })
    }

    fn mark_object_deleted<'a>(
        &'a self,
        _claim: &'a AccountObjectDeletionClaim,
    ) -> AccountObjectDeletionRepositoryFuture<'a, bool> {
        self.success_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(true) })
    }

    fn record_object_deletion_failure<'a>(
        &'a self,
        _claim: &'a AccountObjectDeletionClaim,
        _code: AccountObjectDeletionFailureCode,
        _retry_delay: Duration,
        _max_attempts: u32,
    ) -> AccountObjectDeletionRepositoryFuture<'a, AccountObjectDeletionFailureDisposition> {
        self.retry_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(AccountObjectDeletionFailureDisposition::RetryScheduled) })
    }
}

struct HangingProvider {
    barrier: Barrier,
    starts: AtomicUsize,
}

impl HangingProvider {
    fn new() -> Self {
        Self {
            barrier: Barrier::new(2),
            starts: AtomicUsize::default(),
        }
    }
}

impl AccountObjectDeletionProvider for HangingProvider {
    fn delete_object<'a>(
        &'a self,
        _object_key: &'a str,
    ) -> AccountObjectDeletionProviderFuture<'a> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            self.barrier.wait().await;
            std::future::pending::<Result<(), ObjectStorageProviderError>>().await
        })
    }
}

#[derive(Clone)]
struct ScriptedS3State {
    statuses: Arc<Mutex<VecDeque<StatusCode>>>,
    requests: Arc<Mutex<Vec<ObservedRequest>>>,
}

struct ScriptedS3 {
    endpoint: String,
    state: ScriptedS3State,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<Result<(), io::Error>>,
}

impl ScriptedS3 {
    async fn start(statuses: impl IntoIterator<Item = StatusCode>) -> TestResult<Self> {
        let state = ScriptedS3State {
            statuses: Arc::new(Mutex::new(statuses.into_iter().collect())),
            requests: Arc::new(Mutex::new(Vec::new())),
        };
        let app = Router::new()
            .fallback(any(scripted_s3_response))
            .with_state(state.clone());
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let (shutdown, receiver) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = receiver.await;
                })
                .await
        });
        Ok(Self {
            endpoint: format!("http://{address}"),
            state,
            shutdown: Some(shutdown),
            task,
        })
    }

    fn endpoint(&self) -> &str {
        &self.endpoint
    }

    async fn finish(mut self) -> TestResult<Vec<ObservedRequest>> {
        if let Some(shutdown) = self.shutdown.take() {
            shutdown
                .send(())
                .map_err(|_| io::Error::other("scripted S3 stopped before shutdown"))?;
        }
        (&mut self.task).await??;
        self.state
            .requests
            .lock()
            .map(|requests| requests.clone())
            .map_err(|_| io::Error::other("scripted S3 request mutex is poisoned").into())
    }
}

impl Drop for ScriptedS3 {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ObservedRequest {
    method: String,
    path: String,
    signed: bool,
    uses_cleanup_identity: bool,
    uses_media_identity: bool,
    secret_exposed: bool,
}

async fn scripted_s3_response(
    State(state): State<ScriptedS3State>,
    request: Request<Body>,
) -> Response<Body> {
    let authorization = request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    let observed = ObservedRequest {
        method: request.method().to_string(),
        path: request.uri().path().to_owned(),
        signed: authorization.is_some_and(|value| value.starts_with("AWS4-HMAC-SHA256 ")),
        uses_cleanup_identity: authorization
            .is_some_and(|value| value.contains(&format!("Credential={CLEANUP_ACCESS_KEY_ID}/"))),
        uses_media_identity: authorization
            .is_some_and(|value| value.contains(&format!("Credential={MEDIA_ACCESS_KEY_ID}/"))),
        secret_exposed: authorization.is_some_and(|value| {
            value.contains(SECRET_ACCESS_KEY) || value.contains(MEDIA_SECRET_ACCESS_KEY)
        }),
    };
    let recorded = state
        .requests
        .lock()
        .map(|mut requests| requests.push(observed))
        .is_ok();
    let status = state
        .statuses
        .lock()
        .ok()
        .and_then(|mut statuses| statuses.pop_front())
        .filter(|_| recorded)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    Response::builder()
        .status(status)
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty()))
}
