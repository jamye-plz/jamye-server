use std::{
    future,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use jamye_server::{
    application::push::{PushWorker, PushWorkerConfig, PushWorkerDependencies},
    config::{
        AppConfig, AppEnvironment, ConfigInput,
        push::{PushConfig, PushConfigInput},
    },
    ports::{
        push::{
            ClaimedPushDelivery, ExpoPushDestination, PushDeliveryClaim, PushDeliveryClaimRequest,
            PushDeliveryFailureCode, PushDeliveryFailureDisposition, PushDeliveryRepository,
            PushDeliveryRepositoryFuture, PushInvalidDestinationFuture,
            PushInvalidDestinationRepository, PushPreviewSource, PushPreviewSourceFuture,
            PushProvider, PushProviderError, PushProviderFuture, PushProviderRequest,
            PushRepositoryError, PushSendAuthorizationFuture, PushSendAuthorizationRepository,
        },
        transactions::{
            BoxTransactionHandle, TransactionError, TransactionFuture, TransactionManager,
        },
    },
    transport::push::composition::{WorkerRuntime, worker},
};
use uuid::Uuid;

use super::TestResult;

const DATABASE_PASSWORD: &str = "TASK_9_DATABASE_PASSWORD_SENTINEL";
const EXPO_ACCESS_TOKEN: &str = "TASK_9_EXPO_ACCESS_TOKEN_SENTINEL";

#[tokio::test]
async fn static_composition_wires_validated_config_without_private_state_or_io() -> TestResult {
    let app_config = AppConfig::try_from(ConfigInput {
        environment: Some("test".to_owned()),
        database_url: Some(format!(
            "postgres://task9:{DATABASE_PASSWORD}@127.0.0.1:5432/jamye"
        )),
        ..ConfigInput::default()
    })?;
    let push_config = PushConfig::resolve(
        AppEnvironment::Test,
        PushConfigInput {
            access_token: Some(EXPO_ACCESS_TOKEN.to_owned()),
            poll_interval_ms: Some("37".to_owned()),
            ..PushConfigInput::default()
        },
    )?;

    let runtime = worker(&app_config, &push_config);
    assert_eq!(
        runtime.as_ref().map(|runtime| runtime.poll_interval()),
        Ok(Duration::from_millis(37))
    );
    let debug = format!("{:?}", runtime?);
    assert!(debug.contains("WorkerRuntime"));
    assert!(!debug.contains(DATABASE_PASSWORD));
    assert!(!debug.contains(EXPO_ACCESS_TOKEN));
    Ok(())
}

#[tokio::test]
async fn runtime_polls_once_before_honoring_an_immediate_shutdown() -> TestResult {
    let repository = Arc::new(EmptyRepository::default());
    let worker = PushWorker::new(
        PushWorkerDependencies {
            transactions: Arc::new(UnusedTransactions),
            repository: repository.clone(),
            preview_source: Arc::new(UnusedPreviewSource),
            provider: Arc::new(UnusedProvider),
        },
        PushWorkerConfig {
            claim_owner: "task-9-runtime-test".to_owned(),
            batch_size: 2,
            lease_duration: Duration::from_millis(50),
            provider_timeout: Duration::from_millis(10),
            lease_safety_margin: Duration::from_millis(10),
            retry_delay: Duration::from_millis(5),
            poll_interval: Duration::from_millis(5),
            max_attempts: 3,
        },
    )?;
    let runtime = WorkerRuntime::from_worker(worker);

    assert_eq!(runtime.poll_interval(), Duration::from_millis(5));
    runtime.run_until(future::ready(())).await;
    assert_eq!(repository.claim_count.load(Ordering::SeqCst), 1);
    Ok(())
}

#[derive(Default)]
struct EmptyRepository {
    claim_count: AtomicUsize,
}

impl PushDeliveryRepository for EmptyRepository {
    fn claim_deliveries(
        &self,
        _request: PushDeliveryClaimRequest,
    ) -> PushDeliveryRepositoryFuture<'_, Vec<ClaimedPushDelivery>> {
        self.claim_count.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(Vec::new()) })
    }

    fn mark_delivery_succeeded<'a>(
        &'a self,
        _claim: &'a ClaimedPushDelivery,
    ) -> PushDeliveryRepositoryFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    fn record_delivery_failure<'a>(
        &'a self,
        _claim: &'a ClaimedPushDelivery,
        _code: PushDeliveryFailureCode,
        _retry_delay: Duration,
        _max_attempts: u32,
    ) -> PushDeliveryRepositoryFuture<'a, PushDeliveryFailureDisposition> {
        Box::pin(async { Ok(PushDeliveryFailureDisposition::StaleClaim) })
    }
}

impl PushSendAuthorizationRepository for EmptyRepository {
    fn authorize_send<'a>(
        &'a self,
        _transaction: &'a mut dyn jamye_server::ports::transactions::TransactionHandle,
        _claim: &'a PushDeliveryClaim,
    ) -> PushSendAuthorizationFuture<'a> {
        Box::pin(async { Ok(None) })
    }
}

impl PushInvalidDestinationRepository for EmptyRepository {
    fn disable_invalid_destination<'a>(
        &'a self,
        _transaction: &'a mut dyn jamye_server::ports::transactions::TransactionHandle,
        _claim: &'a PushDeliveryClaim,
        _destination: &'a ExpoPushDestination,
    ) -> PushInvalidDestinationFuture<'a> {
        Box::pin(async { Ok(false) })
    }
}

struct UnusedTransactions;

impl TransactionManager for UnusedTransactions {
    fn begin(&self) -> TransactionFuture<'_, BoxTransactionHandle> {
        Box::pin(async { Err(TransactionError) })
    }

    fn commit<'a>(&'a self, _handle: BoxTransactionHandle) -> TransactionFuture<'a, ()> {
        Box::pin(async { Err(TransactionError) })
    }

    fn rollback<'a>(&'a self, _handle: BoxTransactionHandle) -> TransactionFuture<'a, ()> {
        Box::pin(async { Err(TransactionError) })
    }
}

struct UnusedPreviewSource;

impl PushPreviewSource for UnusedPreviewSource {
    fn load_message_body(&self, _message_id: Uuid) -> PushPreviewSourceFuture<'_> {
        Box::pin(async { Err(PushRepositoryError::Unavailable) })
    }
}

struct UnusedProvider;

impl PushProvider for UnusedProvider {
    fn send<'a>(&'a self, _request: &'a PushProviderRequest) -> PushProviderFuture<'a> {
        Box::pin(async { Err(PushProviderError::Unavailable) })
    }
}
