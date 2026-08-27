use std::{
    any::Any,
    future,
    sync::{Arc, Mutex},
    time::Duration,
};

use jamye_server::{
    application::push::{
        PushWorker, PushWorkerConfig, PushWorkerDependencies, PushWorkerError, PushWorkerReport,
    },
    ports::{
        push::{
            AuthorizedPushDelivery, ClaimedPushDelivery, ExpoPushDestination, NotificationType,
            PushDeliveryClaim, PushDeliveryClaimRequest, PushDeliveryFailureCode,
            PushDeliveryFailureDisposition, PushDeliveryRepository, PushDeliveryRepositoryFuture,
            PushEnvironment, PushInvalidDestinationFuture, PushInvalidDestinationRepository,
            PushPreviewSource, PushPreviewSourceFuture, PushProvider, PushProviderError,
            PushProviderFuture, PushProviderOutcome, PushProviderRequest, PushRepositoryError,
            PushSendAuthorizationFuture, PushSendAuthorizationRepository, PushTapPayload,
        },
        transactions::{
            BoxTransactionHandle, TransactionFuture, TransactionHandle, TransactionManager,
        },
    },
};
use time::OffsetDateTime;
use uuid::Uuid;

const EXPO_TOKEN: &str = "ExponentPushToken[task-9-worker-secret]";

#[test]
fn worker_requires_a_strict_provider_budget_inside_the_lease_without_side_effects() {
    let harness = Harness::new(
        Ok(Some(authorized_delivery(None))),
        Ok(None),
        ProviderBehavior::Outcome(PushProviderOutcome::Accepted),
        Ok(PushDeliveryFailureDisposition::RetryScheduled),
        Ok(true),
    );
    let mut equal_budget = worker_config();
    equal_budget.lease_duration = equal_budget.provider_timeout + equal_budget.lease_safety_margin;
    let mut control_owner = worker_config();
    control_owner.claim_owner = "worker\nowner".to_owned();
    let mut zero_batch = worker_config();
    zero_batch.batch_size = 0;

    for config in [equal_budget, control_owner, zero_batch] {
        assert_eq!(
            PushWorker::new(harness.dependencies.clone(), config).err(),
            Some(PushWorkerError::InvalidConfiguration)
        );
    }
    assert_eq!(harness.worker.poll_interval(), Duration::from_millis(5));
    assert_eq!(harness.calls(), Vec::<Call>::new());
}

#[tokio::test]
async fn authorization_commits_before_preview_and_provider_then_acceptance_completes_only_the_claim()
 {
    let harness = Harness::new(
        Ok(Some(authorized_delivery(Some(message_id())))),
        Ok(Some("  친구가\n\t보낸   메시지  ".to_owned())),
        ProviderBehavior::Outcome(PushProviderOutcome::Accepted),
        Ok(PushDeliveryFailureDisposition::RetryScheduled),
        Ok(true),
    );

    assert_eq!(
        harness.worker.run_once().await,
        Ok(PushWorkerReport {
            claimed: 1,
            succeeded: 1,
            ..PushWorkerReport::default()
        })
    );
    assert_eq!(
        harness.calls(),
        vec![
            Call::Claim,
            Call::Begin,
            Call::Authorize,
            Call::Commit,
            Call::Preview,
            Call::Provider,
            Call::MarkSucceeded,
        ]
    );
    assert_eq!(
        harness.repository.claim_requests(),
        vec![PushDeliveryClaimRequest {
            claim_owner: "task-9-worker".to_owned(),
            batch_size: 1,
            lease_duration: Duration::from_millis(50),
        }]
    );
    assert_eq!(harness.preview_source.message_ids(), vec![message_id()]);
    let requests = harness.provider.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].route, route());
    assert_eq!(requests[0].preview.as_deref(), Some("친구가 보낸 메시지"));
    let debug = format!("{:?}", requests[0]);
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains(EXPO_TOKEN));
    assert!(!debug.contains("친구가 보낸 메시지"));

    let bounded = Harness::new(
        Ok(Some(authorized_delivery(Some(message_id())))),
        Ok(Some("가".repeat(81))),
        ProviderBehavior::Outcome(PushProviderOutcome::Accepted),
        Ok(PushDeliveryFailureDisposition::RetryScheduled),
        Ok(true),
    );
    assert!(bounded.worker.run_once().await.is_ok());
    let bounded_requests = bounded.provider.requests();
    let preview = bounded_requests[0].preview.as_deref();
    let expected_preview = "가".repeat(80);
    assert_eq!(preview, Some(expected_preview.as_str()));
    assert_eq!(preview.map(|value| value.chars().count()), Some(80));

    let media_only = Harness::new(
        Ok(Some(authorized_delivery(Some(message_id())))),
        Ok(None),
        ProviderBehavior::Outcome(PushProviderOutcome::Accepted),
        Ok(PushDeliveryFailureDisposition::RetryScheduled),
        Ok(true),
    );
    assert!(media_only.worker.run_once().await.is_ok());
    assert_eq!(media_only.provider.requests()[0].preview, None);
}

#[tokio::test]
async fn denied_authorization_rolls_back_without_preview_or_provider_and_terminalizes_the_claim() {
    let harness = Harness::new(
        Ok(None),
        Ok(Some("절대 조회하면 안 되는 본문".to_owned())),
        ProviderBehavior::Outcome(PushProviderOutcome::Accepted),
        Ok(PushDeliveryFailureDisposition::DeadLettered),
        Ok(true),
    );

    assert_eq!(
        harness.worker.run_once().await,
        Ok(PushWorkerReport {
            claimed: 1,
            dead_lettered: 1,
            authorization_denied: 1,
            ..PushWorkerReport::default()
        })
    );
    assert_eq!(
        harness.calls(),
        vec![
            Call::Claim,
            Call::Begin,
            Call::Authorize,
            Call::Rollback,
            Call::RecordFailure(PushDeliveryFailureCode::AuthorizationDenied),
        ]
    );
    assert!(harness.preview_source.message_ids().is_empty());
    assert!(harness.provider.requests().is_empty());
    assert_eq!(
        harness.repository.failures(),
        vec![(
            PushDeliveryFailureCode::AuthorizationDenied,
            Duration::from_millis(5),
            1,
        )]
    );
}

#[tokio::test]
async fn preview_outage_and_expo_outage_or_timeout_remain_retryable_without_false_success() {
    let cases = [
        (
            Ok(Some(authorized_delivery(Some(message_id())))),
            Err(PushRepositoryError::Unavailable),
            ProviderBehavior::Outcome(PushProviderOutcome::Accepted),
            PushDeliveryFailureCode::PreviewUnavailable,
        ),
        (
            Ok(Some(authorized_delivery(None))),
            Ok(None),
            ProviderBehavior::Error(PushProviderError::Unavailable),
            PushDeliveryFailureCode::ExpoUnavailable,
        ),
        (
            Ok(Some(authorized_delivery(None))),
            Ok(None),
            ProviderBehavior::Pending,
            PushDeliveryFailureCode::ExpoTimeout,
        ),
    ];

    for (authorization, preview, provider, expected_code) in cases {
        let harness = Harness::new(
            authorization,
            preview,
            provider,
            Ok(PushDeliveryFailureDisposition::RetryScheduled),
            Ok(true),
        );
        assert_eq!(
            harness.worker.run_once().await,
            Ok(PushWorkerReport {
                claimed: 1,
                retries: 1,
                ..PushWorkerReport::default()
            })
        );
        assert_eq!(
            harness.repository.failures(),
            vec![(expected_code, Duration::from_millis(5), 3)]
        );
        assert!(!harness.calls().contains(&Call::MarkSucceeded));
        if expected_code == PushDeliveryFailureCode::PreviewUnavailable {
            assert!(harness.provider.requests().is_empty());
        }
    }
}

#[tokio::test]
async fn non_retryable_provider_rejection_dead_letters_without_false_success() {
    let harness = Harness::new(
        Ok(Some(authorized_delivery(None))),
        Ok(None),
        ProviderBehavior::Error(PushProviderError::Rejected),
        Ok(PushDeliveryFailureDisposition::DeadLettered),
        Ok(true),
    );

    assert_eq!(
        harness.worker.run_once().await,
        Ok(PushWorkerReport {
            claimed: 1,
            dead_lettered: 1,
            ..PushWorkerReport::default()
        })
    );
    assert_eq!(
        harness.repository.failures(),
        vec![(
            PushDeliveryFailureCode::ExpoRejected,
            Duration::from_millis(5),
            1,
        )]
    );
    assert!(!harness.calls().contains(&Call::MarkSucceeded));
}

#[tokio::test]
async fn device_not_registered_disables_only_the_exact_current_destination_and_stale_feedback_rolls_back()
 {
    for (disable_result, expected_report, final_call) in [
        (
            Ok(true),
            PushWorkerReport {
                claimed: 1,
                invalid_destinations: 1,
                ..PushWorkerReport::default()
            },
            Call::Commit,
        ),
        (
            Ok(false),
            PushWorkerReport {
                claimed: 1,
                stale_claims: 1,
                ..PushWorkerReport::default()
            },
            Call::Rollback,
        ),
    ] {
        let harness = Harness::new(
            Ok(Some(authorized_delivery(None))),
            Ok(None),
            ProviderBehavior::Outcome(PushProviderOutcome::DeviceNotRegistered),
            Ok(PushDeliveryFailureDisposition::RetryScheduled),
            disable_result,
        );
        assert_eq!(harness.worker.run_once().await, Ok(expected_report));
        assert_eq!(
            harness.calls(),
            vec![
                Call::Claim,
                Call::Begin,
                Call::Authorize,
                Call::Commit,
                Call::Provider,
                Call::Begin,
                Call::DisableInvalidDestination,
                final_call,
            ]
        );
        assert_eq!(
            harness.repository.invalid_destinations(),
            vec![(claim(), destination())]
        );
        assert!(!harness.calls().contains(&Call::MarkSucceeded));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Call {
    Claim,
    Begin,
    Authorize,
    Commit,
    Rollback,
    Preview,
    Provider,
    MarkSucceeded,
    RecordFailure(PushDeliveryFailureCode),
    DisableInvalidDestination,
}

struct Harness {
    worker: PushWorker,
    dependencies: PushWorkerDependencies,
    calls: Arc<Mutex<Vec<Call>>>,
    repository: Arc<RecordingRepository>,
    preview_source: Arc<RecordingPreviewSource>,
    provider: Arc<RecordingProvider>,
}

impl Harness {
    fn new(
        authorization_result: Result<Option<AuthorizedPushDelivery>, PushRepositoryError>,
        preview_result: Result<Option<String>, PushRepositoryError>,
        provider_behavior: ProviderBehavior,
        failure_result: Result<PushDeliveryFailureDisposition, PushRepositoryError>,
        disable_result: Result<bool, PushRepositoryError>,
    ) -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let repository = Arc::new(RecordingRepository {
            calls: calls.clone(),
            claim_requests: Mutex::new(Vec::new()),
            failures: Mutex::new(Vec::new()),
            invalid_destinations: Mutex::new(Vec::new()),
            authorization_result,
            failure_result,
            disable_result,
        });
        let preview_source = Arc::new(RecordingPreviewSource {
            calls: calls.clone(),
            message_ids: Mutex::new(Vec::new()),
            result: preview_result,
        });
        let provider = Arc::new(RecordingProvider {
            calls: calls.clone(),
            requests: Mutex::new(Vec::new()),
            behavior: provider_behavior,
        });
        let dependencies = PushWorkerDependencies {
            transactions: Arc::new(RecordingTransactions {
                calls: calls.clone(),
            }),
            repository: repository.clone(),
            preview_source: preview_source.clone(),
            provider: provider.clone(),
        };
        let worker = match PushWorker::new(dependencies.clone(), worker_config()) {
            Ok(worker) => worker,
            Err(error) => panic!("test worker configuration must remain valid: {error:?}"),
        };
        Self {
            worker,
            dependencies,
            calls,
            repository,
            preview_source,
            provider,
        }
    }

    fn calls(&self) -> Vec<Call> {
        crate::lock_test_mutex(&self.calls, "worker call").clone()
    }
}

struct RecordingHandle;

impl TransactionHandle for RecordingHandle {
    fn as_any_mut(&mut self) -> &mut (dyn Any + Send) {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }
}

struct RecordingTransactions {
    calls: Arc<Mutex<Vec<Call>>>,
}

impl TransactionManager for RecordingTransactions {
    fn begin(&self) -> TransactionFuture<'_, BoxTransactionHandle> {
        record(&self.calls, Call::Begin);
        Box::pin(async { Ok(Box::new(RecordingHandle) as BoxTransactionHandle) })
    }

    fn commit<'a>(&'a self, handle: BoxTransactionHandle) -> TransactionFuture<'a, ()> {
        assert!(handle.into_any().downcast::<RecordingHandle>().is_ok());
        record(&self.calls, Call::Commit);
        Box::pin(async { Ok(()) })
    }

    fn rollback<'a>(&'a self, handle: BoxTransactionHandle) -> TransactionFuture<'a, ()> {
        assert!(handle.into_any().downcast::<RecordingHandle>().is_ok());
        record(&self.calls, Call::Rollback);
        Box::pin(async { Ok(()) })
    }
}

struct RecordingRepository {
    calls: Arc<Mutex<Vec<Call>>>,
    claim_requests: Mutex<Vec<PushDeliveryClaimRequest>>,
    failures: Mutex<Vec<(PushDeliveryFailureCode, Duration, u32)>>,
    invalid_destinations: Mutex<Vec<(PushDeliveryClaim, ExpoPushDestination)>>,
    authorization_result: Result<Option<AuthorizedPushDelivery>, PushRepositoryError>,
    failure_result: Result<PushDeliveryFailureDisposition, PushRepositoryError>,
    disable_result: Result<bool, PushRepositoryError>,
}

impl RecordingRepository {
    fn claim_requests(&self) -> Vec<PushDeliveryClaimRequest> {
        crate::lock_test_mutex(&self.claim_requests, "claim request").clone()
    }

    fn failures(&self) -> Vec<(PushDeliveryFailureCode, Duration, u32)> {
        crate::lock_test_mutex(&self.failures, "delivery failure").clone()
    }

    fn invalid_destinations(&self) -> Vec<(PushDeliveryClaim, ExpoPushDestination)> {
        crate::lock_test_mutex(&self.invalid_destinations, "invalid destination").clone()
    }
}

impl PushDeliveryRepository for RecordingRepository {
    fn claim_deliveries(
        &self,
        request: PushDeliveryClaimRequest,
    ) -> PushDeliveryRepositoryFuture<'_, Vec<ClaimedPushDelivery>> {
        record(&self.calls, Call::Claim);
        crate::lock_test_mutex(&self.claim_requests, "claim request").push(request);
        Box::pin(async { Ok(vec![claimed_delivery()]) })
    }

    fn mark_delivery_succeeded<'a>(
        &'a self,
        _claim: &'a ClaimedPushDelivery,
    ) -> PushDeliveryRepositoryFuture<'a, bool> {
        record(&self.calls, Call::MarkSucceeded);
        Box::pin(async { Ok(true) })
    }

    fn record_delivery_failure<'a>(
        &'a self,
        _claim: &'a ClaimedPushDelivery,
        code: PushDeliveryFailureCode,
        retry_delay: Duration,
        max_attempts: u32,
    ) -> PushDeliveryRepositoryFuture<'a, PushDeliveryFailureDisposition> {
        record(&self.calls, Call::RecordFailure(code));
        crate::lock_test_mutex(&self.failures, "delivery failure").push((
            code,
            retry_delay,
            max_attempts,
        ));
        let result = self.failure_result;
        Box::pin(async move { result })
    }
}

impl PushSendAuthorizationRepository for RecordingRepository {
    fn authorize_send<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _claim: &'a PushDeliveryClaim,
    ) -> PushSendAuthorizationFuture<'a> {
        record(&self.calls, Call::Authorize);
        let result = self.authorization_result.clone();
        Box::pin(async move { result })
    }
}

impl PushInvalidDestinationRepository for RecordingRepository {
    fn disable_invalid_destination<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        claim: &'a PushDeliveryClaim,
        destination: &'a ExpoPushDestination,
    ) -> PushInvalidDestinationFuture<'a> {
        record(&self.calls, Call::DisableInvalidDestination);
        crate::lock_test_mutex(&self.invalid_destinations, "invalid destination")
            .push((claim.clone(), destination.clone()));
        let result = self.disable_result;
        Box::pin(async move { result })
    }
}

struct RecordingPreviewSource {
    calls: Arc<Mutex<Vec<Call>>>,
    message_ids: Mutex<Vec<Uuid>>,
    result: Result<Option<String>, PushRepositoryError>,
}

impl RecordingPreviewSource {
    fn message_ids(&self) -> Vec<Uuid> {
        crate::lock_test_mutex(&self.message_ids, "preview message id").clone()
    }
}

impl PushPreviewSource for RecordingPreviewSource {
    fn load_message_body(&self, message_id: Uuid) -> PushPreviewSourceFuture<'_> {
        record(&self.calls, Call::Preview);
        crate::lock_test_mutex(&self.message_ids, "preview message id").push(message_id);
        let result = self.result.clone();
        Box::pin(async move { result })
    }
}

#[derive(Clone, Copy)]
enum ProviderBehavior {
    Outcome(PushProviderOutcome),
    Error(PushProviderError),
    Pending,
}

struct RecordingProvider {
    calls: Arc<Mutex<Vec<Call>>>,
    requests: Mutex<Vec<PushProviderRequest>>,
    behavior: ProviderBehavior,
}

impl RecordingProvider {
    fn requests(&self) -> Vec<PushProviderRequest> {
        crate::lock_test_mutex(&self.requests, "provider request").clone()
    }
}

impl PushProvider for RecordingProvider {
    fn send<'a>(&'a self, request: &'a PushProviderRequest) -> PushProviderFuture<'a> {
        record(&self.calls, Call::Provider);
        crate::lock_test_mutex(&self.requests, "provider request").push(request.clone());
        let behavior = self.behavior;
        Box::pin(async move {
            match behavior {
                ProviderBehavior::Outcome(outcome) => Ok(outcome),
                ProviderBehavior::Error(error) => Err(error),
                ProviderBehavior::Pending => future::pending().await,
            }
        })
    }
}

fn record(calls: &Mutex<Vec<Call>>, call: Call) {
    crate::lock_test_mutex(calls, "worker call").push(call);
}

fn worker_config() -> PushWorkerConfig {
    PushWorkerConfig {
        claim_owner: "task-9-worker".to_owned(),
        batch_size: 1,
        lease_duration: Duration::from_millis(50),
        provider_timeout: Duration::from_millis(10),
        lease_safety_margin: Duration::from_millis(10),
        retry_delay: Duration::from_millis(5),
        poll_interval: Duration::from_millis(5),
        max_attempts: 3,
    }
}

fn claimed_delivery() -> ClaimedPushDelivery {
    ClaimedPushDelivery {
        claim: claim(),
        claim_expires_at: OffsetDateTime::UNIX_EPOCH,
        attempt_count: 1,
    }
}

fn claim() -> PushDeliveryClaim {
    PushDeliveryClaim {
        occurrence_id: occurrence_id(),
        claim_owner: "task-9-worker".to_owned(),
        claim_generation: 1,
    }
}

fn authorized_delivery(preview_message_id: Option<Uuid>) -> AuthorizedPushDelivery {
    AuthorizedPushDelivery {
        occurrence_id: occurrence_id(),
        route: route(),
        destination: destination(),
        preview_message_id,
    }
}

fn route() -> PushTapPayload {
    PushTapPayload {
        notification_type: NotificationType::ChatUnread,
        notification_id: notification_id(),
        conversation_id: conversation_id(),
        message_id: Some(message_id()),
    }
}

fn destination() -> ExpoPushDestination {
    ExpoPushDestination::new(PushEnvironment::Development, EXPO_TOKEN.to_owned())
}

fn occurrence_id() -> Uuid {
    Uuid::from_u128(0x11111111_1111_4111_8111_111111111111)
}

fn notification_id() -> Uuid {
    Uuid::from_u128(0x22222222_2222_4222_8222_222222222222)
}

fn conversation_id() -> Uuid {
    Uuid::from_u128(0x33333333_3333_4333_8333_333333333333)
}

fn message_id() -> Uuid {
    Uuid::from_u128(0x44444444_4444_4444_8444_444444444444)
}
