use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use jamye_server::{
    adapters::postgres::{push::PostgresPushRepository, transactions::SqlxTransactionManager},
    application::push::{PushWorker, PushWorkerConfig, PushWorkerDependencies, PushWorkerReport},
    ports::{
        push::{
            ClaimedPushDelivery, ExpoPushDestination, PushDeliveryClaim, PushDeliveryClaimRequest,
            PushDeliveryFailureCode, PushDeliveryFailureDisposition, PushDeliveryRepository,
            PushDeliveryRepositoryFuture, PushInvalidDestinationFuture,
            PushInvalidDestinationRepository, PushPreviewSource, PushPreviewSourceFuture,
            PushProvider, PushProviderFuture, PushProviderOutcome, PushSendAuthorizationFuture,
            PushSendAuthorizationRepository,
        },
        transactions::TransactionHandle,
    },
};
use sqlx::PgPool;
use tokio::{sync::Notify, time::timeout};
use tower::ServiceExt;

use crate::{
    TestResult,
    postgres_support::TestDatabase,
    send_topology_support::SendTopology,
    support::{delete_request, finish_database_test, require_eq, test_error, test_router},
};

const BARRIER_TIMEOUT: Duration = Duration::from_secs(3);

#[tokio::test]
async fn deletion_before_claim_prevents_preview_provider_retry_and_reclaim() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let topology = pending_topology(&pool).await?;
        let provider = Arc::new(CountingProvider::default());
        let preview = Arc::new(CountingPreview::new(pool.clone()));
        let worker = worker(
            pool.clone(),
            Arc::new(GatedRepository::new(pool.clone(), None)),
            preview.clone(),
            provider.clone(),
        )?;

        let _deletion = test_router(pool.clone())?
            .oneshot(delete_request(topology.recipient_id)?)
            .await?;
        let report = worker.run_once().await?;

        require_eq(provider.calls(), 0, "delete-before-claim called provider")?;
        require_eq(preview.calls(), 0, "delete-before-claim loaded preview")?;
        require_eq(report.claimed, 0, "delete-before-claim still claimed push")?;
        require_eq(report.retries, 0, "delete-before-claim scheduled retry")?;
        require_eq(
            reclaimable(&pool, topology.occurrence_id).await?,
            0,
            "delete-before-claim left reclaimable push",
        )?;

        Ok(())
    }
    .await;
    finish_database_test(database, pool, result).await
}

#[tokio::test]
async fn deletion_after_claim_before_authorization_denies_send_without_retry_or_stale_reclaim()
-> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let topology = pending_topology(&pool).await?;
        let gate = Arc::new(AuthorizationGate::default());
        let provider = Arc::new(CountingProvider::default());
        let preview = Arc::new(CountingPreview::new(pool.clone()));
        let worker = worker(
            pool.clone(),
            Arc::new(GatedRepository::new(pool.clone(), Some(gate.clone()))),
            preview.clone(),
            provider.clone(),
        )?;
        let task = tokio::spawn(async move { worker.run_once().await });

        if let Err(error) = wait_for(
            &gate.entered,
            "push authorization did not reach its test gate",
        )
        .await
        {
            task.abort();
            let _aborted = task.await;
            return Err(error);
        }
        let _deletion = test_router(pool.clone())?
            .oneshot(delete_request(topology.recipient_id)?)
            .await?;
        gate.released.notify_one();
        let report = join_worker(task, "claim-before-authorization worker did not finish").await?;

        require_eq(provider.calls(), 0, "claim-before-auth called provider")?;
        require_eq(preview.calls(), 0, "claim-before-auth loaded preview")?;
        require_eq(report.retries, 0, "claim-before-auth scheduled retry")?;
        require_eq(report.succeeded, 0, "claim-before-auth reported success")?;
        require_eq(
            report.authorization_denied,
            1,
            "claim-before-auth did not deny authorization",
        )?;
        require_eq(
            report.stale_claims,
            1,
            "claim-before-auth did not fence its late failure CAS",
        )?;
        require_eq(
            reclaimable(&pool, topology.occurrence_id).await?,
            0,
            "claim-before-auth left reclaimable push",
        )?;

        Ok(())
    }
    .await;
    finish_database_test(database, pool, result).await
}

#[tokio::test]
async fn deletion_after_provider_start_allows_at_most_one_call_and_fences_late_completion()
-> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let topology = pending_topology(&pool).await?;
        let provider = Arc::new(BlockingProvider::default());
        let preview = Arc::new(CountingPreview::new(pool.clone()));
        let repository = Arc::new(GatedRepository::new(pool.clone(), None));
        let worker = worker(pool.clone(), repository, preview.clone(), provider.clone())?;
        let first_worker = worker.clone();
        let task = tokio::spawn(async move { first_worker.run_once().await });

        if let Err(error) = wait_for(
            &provider.started,
            "push provider did not reach its test gate",
        )
        .await
        {
            task.abort();
            let _aborted = task.await;
            return Err(error);
        }
        let _deletion = test_router(pool.clone())?
            .oneshot(delete_request(topology.recipient_id)?)
            .await?;
        provider.released.notify_one();
        let report = join_worker(task, "authorization-first worker did not finish").await?;

        require_eq(
            provider.calls(),
            1,
            "authorization-first provider call count differed",
        )?;
        require_eq(
            preview.calls(),
            1,
            "authorization-first preview count differed",
        )?;
        require_eq(
            report.succeeded,
            0,
            "late provider result produced false success",
        )?;
        require_eq(report.retries, 0, "late provider result scheduled retry")?;
        require_eq(
            report.stale_claims,
            1,
            "late provider completion was not generation fenced",
        )?;
        require_eq(
            reclaimable(&pool, topology.occurrence_id).await?,
            0,
            "authorization-first path left reclaimable push",
        )?;

        let later = worker.run_once().await?;
        require_eq(
            later,
            PushWorkerReport::default(),
            "later worker poll observed deleted push state",
        )?;
        require_eq(provider.calls(), 1, "later poll called provider again")?;
        require_eq(preview.calls(), 1, "later poll loaded preview again")?;

        Ok(())
    }
    .await;
    finish_database_test(database, pool, result).await
}

async fn pending_topology(pool: &PgPool) -> TestResult<SendTopology> {
    let topology = SendTopology::new(pool).await?;
    sqlx::query(
        "UPDATE push_delivery_intents \
         SET status = 'pending', claim_owner = NULL, claim_generation = 0, \
             lease_expires_at = NULL, attempt_count = 0, next_attempt_at = NULL, \
             last_error_code = NULL \
         WHERE id = $1",
    )
    .bind(topology.occurrence_id)
    .execute(pool)
    .await?;
    Ok(topology)
}

fn worker(
    pool: PgPool,
    repository: Arc<GatedRepository>,
    preview: Arc<CountingPreview>,
    provider: Arc<dyn PushProvider>,
) -> TestResult<PushWorker> {
    Ok(PushWorker::new(
        PushWorkerDependencies {
            transactions: Arc::new(SqlxTransactionManager::new(pool)),
            repository,
            preview_source: preview,
            provider,
        },
        PushWorkerConfig {
            claim_owner: "task-11-deletion-barrier".to_owned(),
            batch_size: 1,
            lease_duration: Duration::from_secs(10),
            provider_timeout: Duration::from_secs(5),
            lease_safety_margin: Duration::from_secs(1),
            retry_delay: Duration::from_secs(1),
            poll_interval: Duration::from_millis(10),
            max_attempts: 3,
        },
    )?)
}

async fn reclaimable(pool: &PgPool, occurrence_id: uuid::Uuid) -> TestResult<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM push_delivery_intents \
         WHERE id = $1 AND status IN ('pending', 'claimed', 'retryable')",
    )
    .bind(occurrence_id)
    .fetch_one(pool)
    .await?)
}

async fn wait_for(notify: &Notify, message: &str) -> TestResult {
    timeout(BARRIER_TIMEOUT, notify.notified())
        .await
        .map_err(|_| test_error(message))?;
    Ok(())
}

async fn join_worker(
    mut task: tokio::task::JoinHandle<
        Result<PushWorkerReport, jamye_server::application::push::PushWorkerError>,
    >,
    message: &str,
) -> TestResult<PushWorkerReport> {
    match timeout(BARRIER_TIMEOUT, &mut task).await {
        Ok(joined) => Ok(joined??),
        Err(_) => {
            task.abort();
            let _aborted = task.await;
            Err(test_error(message))
        }
    }
}

#[derive(Default)]
struct AuthorizationGate {
    entered: Notify,
    released: Notify,
}

struct GatedRepository {
    inner: PostgresPushRepository,
    authorization_gate: Option<Arc<AuthorizationGate>>,
}

impl GatedRepository {
    fn new(pool: PgPool, authorization_gate: Option<Arc<AuthorizationGate>>) -> Self {
        Self {
            inner: PostgresPushRepository::new(pool),
            authorization_gate,
        }
    }
}

impl PushDeliveryRepository for GatedRepository {
    fn claim_deliveries(
        &self,
        request: PushDeliveryClaimRequest,
    ) -> PushDeliveryRepositoryFuture<'_, Vec<ClaimedPushDelivery>> {
        self.inner.claim_deliveries(request)
    }

    fn mark_delivery_succeeded<'a>(
        &'a self,
        claim: &'a ClaimedPushDelivery,
    ) -> PushDeliveryRepositoryFuture<'a, bool> {
        self.inner.mark_delivery_succeeded(claim)
    }

    fn record_delivery_failure<'a>(
        &'a self,
        claim: &'a ClaimedPushDelivery,
        code: PushDeliveryFailureCode,
        retry_delay: Duration,
        max_attempts: u32,
    ) -> PushDeliveryRepositoryFuture<'a, PushDeliveryFailureDisposition> {
        self.inner
            .record_delivery_failure(claim, code, retry_delay, max_attempts)
    }
}

impl PushSendAuthorizationRepository for GatedRepository {
    fn authorize_send<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        claim: &'a PushDeliveryClaim,
    ) -> PushSendAuthorizationFuture<'a> {
        Box::pin(async move {
            if let Some(gate) = &self.authorization_gate {
                gate.entered.notify_one();
                gate.released.notified().await;
            }
            self.inner.authorize_send(transaction, claim).await
        })
    }
}

impl PushInvalidDestinationRepository for GatedRepository {
    fn disable_invalid_destination<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        claim: &'a PushDeliveryClaim,
        destination: &'a ExpoPushDestination,
    ) -> PushInvalidDestinationFuture<'a> {
        self.inner
            .disable_invalid_destination(transaction, claim, destination)
    }
}

struct CountingPreview {
    inner: PostgresPushRepository,
    calls: AtomicUsize,
}

impl CountingPreview {
    fn new(pool: PgPool) -> Self {
        Self {
            inner: PostgresPushRepository::new(pool),
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl PushPreviewSource for CountingPreview {
    fn load_message_body(&self, message_id: uuid::Uuid) -> PushPreviewSourceFuture<'_> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.load_message_body(message_id)
    }
}

#[derive(Default)]
struct CountingProvider {
    calls: AtomicUsize,
}

impl CountingProvider {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl PushProvider for CountingProvider {
    fn send<'a>(
        &'a self,
        _request: &'a jamye_server::ports::push::PushProviderRequest,
    ) -> PushProviderFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(PushProviderOutcome::Accepted) })
    }
}

#[derive(Default)]
struct BlockingProvider {
    calls: AtomicUsize,
    started: Notify,
    released: Notify,
}

impl BlockingProvider {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl PushProvider for BlockingProvider {
    fn send<'a>(
        &'a self,
        _request: &'a jamye_server::ports::push::PushProviderRequest,
    ) -> PushProviderFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            self.started.notify_one();
            self.released.notified().await;
            Ok(PushProviderOutcome::Accepted)
        })
    }
}
