use std::{
    any::Any,
    sync::{Arc, Mutex},
    time::Duration,
};

use jamye_server::{
    application::media::{
        MediaDependencies, MediaEndpointRateLimit, MediaError, MediaService,
        UploadIntentCreateInput,
    },
    domain::media::{InspectedObject, MediaKind, MediaScope, PRESIGNED_PUT_TTL_SECONDS},
    ports::{
        media::{
            CreateUploadIntentCommand, FinalizeUploadCommand, MediaRepository,
            MediaRepositoryError, MediaRepositoryFuture, PrepareUploadFinalizeQuery,
            UploadFinalizePreparation, UploadFinalizeRecord, UploadIntentRecord,
        },
        object_storage::{
            InspectObjectRequest, MediaObjectStorage, MediaObjectStorageFuture,
            ObjectStorageProviderError, PresignPutRequest, PresignedPut,
        },
        rate_limit::{
            RateLimitError, RateLimitFuture, RateLimitOutcome, RateLimitRequest, RateLimiter,
        },
        transactions::{
            BoxTransactionHandle, TransactionFuture, TransactionHandle, TransactionManager,
        },
    },
};
use time::OffsetDateTime;
use uuid::Uuid;

#[tokio::test]
async fn invalid_upload_policy_fails_before_rate_limit_or_any_side_effect() {
    let harness = Harness::new(
        Ok(RateLimitOutcome::Allowed),
        RepositoryMode::Success,
        StorageMode::Success,
    );
    let mut input = input(MediaScope::Chat);
    input.content_type = "application/pdf".to_owned();

    assert_eq!(
        harness
            .service
            .create_upload_intent(actor_id(), input)
            .await,
        Err(MediaError::RequestValidation)
    );
    assert_eq!(harness.calls(), Vec::<Call>::new());
}

#[tokio::test]
async fn rate_limit_denial_and_outage_fail_closed_before_transaction_or_presign() {
    let denied = Harness::new(
        Ok(RateLimitOutcome::Denied {
            retry_after: Duration::from_secs(7),
        }),
        RepositoryMode::Success,
        StorageMode::Success,
    );
    assert_eq!(
        denied
            .service
            .create_upload_intent(actor_id(), input(MediaScope::Chat))
            .await,
        Err(MediaError::RateLimited {
            retry_after: Duration::from_secs(7),
        })
    );
    assert_eq!(denied.calls(), vec![Call::RateLimit]);

    let unavailable = Harness::new(
        Err(RateLimitError),
        RepositoryMode::Success,
        StorageMode::Success,
    );
    assert_eq!(
        unavailable
            .service
            .create_upload_intent(actor_id(), input(MediaScope::Topic))
            .await,
        Err(MediaError::RateLimitUnavailable)
    );
    assert_eq!(unavailable.calls(), vec![Call::RateLimit]);
}

#[tokio::test]
async fn successful_intent_orders_rate_limit_insert_presign_and_one_commit() {
    let harness = Harness::new(
        Ok(RateLimitOutcome::Allowed),
        RepositoryMode::Success,
        StorageMode::Success,
    );
    let actor_id = actor_id();
    let target_id = target_id();
    let Ok(result) = harness
        .service
        .create_upload_intent(
            actor_id,
            UploadIntentCreateInput {
                scope: MediaScope::Chat,
                target_id,
                content_type: "audio/mp4".to_owned(),
                byte_size: 1_024,
                filename: Some(" 음성/메모.m4a ".to_owned()),
            },
        )
        .await
    else {
        panic!("upload intent should be created");
    };

    assert_eq!(
        harness.calls(),
        vec![
            Call::RateLimit,
            Call::Begin,
            Call::Repository,
            Call::Presign,
            Call::Commit,
        ]
    );
    let requests = harness.rate_limiter.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].endpoint, "media_upload_presign");
    assert_eq!(
        requests[0].subject,
        format!("user:{actor_id}:scope:chat:target:{target_id}")
    );
    assert_eq!(requests[0].limit, 2);
    assert_eq!(requests[0].window, Duration::from_secs(60));

    let commands = harness.repository.commands();
    assert_eq!(commands.len(), 1);
    let command = &commands[0];
    assert_eq!(command.id, result.upload.id);
    assert_eq!(command.user_id, actor_id);
    assert_eq!(command.scope, MediaScope::Chat);
    assert_eq!(command.target_id, target_id);
    assert_eq!(command.kind, MediaKind::Audio);
    assert_eq!(command.content_type, "audio/mp4");
    assert_eq!(command.byte_size, 1_024);
    assert_eq!(command.filename.as_deref(), Some(" 음성/메모.m4a "));
    assert_eq!(
        command.expires_in,
        Duration::from_secs(PRESIGNED_PUT_TTL_SECONDS)
    );
    assert_eq!(
        command.object_key,
        format!("chat/{target_id}/{}", result.upload.id)
    );

    let presigns = harness.object_storage.requests();
    assert_eq!(presigns.len(), 1);
    assert_eq!(presigns[0].object_key, command.object_key);
    assert_eq!(presigns[0].content_type, command.content_type);
    assert_eq!(presigns[0].byte_size, command.byte_size);
    assert_eq!(presigns[0].expires_in, command.expires_in);
    assert_eq!(result.put.expires_in, command.expires_in);
}

#[tokio::test]
async fn authorization_or_presign_failure_rolls_back_without_a_commit() {
    let inaccessible = Harness::new(
        Ok(RateLimitOutcome::Allowed),
        RepositoryMode::TargetNotAccessible,
        StorageMode::Success,
    );
    assert_eq!(
        inaccessible
            .service
            .create_upload_intent(actor_id(), input(MediaScope::Chat))
            .await,
        Err(MediaError::TargetNotAccessible)
    );
    assert_eq!(
        inaccessible.calls(),
        vec![
            Call::RateLimit,
            Call::Begin,
            Call::Repository,
            Call::Rollback,
        ]
    );

    let degraded = Harness::new(
        Ok(RateLimitOutcome::Allowed),
        RepositoryMode::Success,
        StorageMode::Unavailable,
    );
    assert_eq!(
        degraded
            .service
            .create_upload_intent(actor_id(), input(MediaScope::Topic))
            .await,
        Err(MediaError::ObjectStorageDegraded)
    );
    assert_eq!(
        degraded.calls(),
        vec![
            Call::RateLimit,
            Call::Begin,
            Call::Repository,
            Call::Presign,
            Call::Rollback,
        ]
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Call {
    RateLimit,
    Begin,
    Repository,
    Presign,
    Commit,
    Rollback,
}

struct Harness {
    service: MediaService,
    calls: Arc<Mutex<Vec<Call>>>,
    rate_limiter: Arc<RecordingRateLimiter>,
    repository: Arc<RecordingRepository>,
    object_storage: Arc<RecordingObjectStorage>,
}

impl Harness {
    fn new(
        rate_limit_result: Result<RateLimitOutcome, RateLimitError>,
        repository_mode: RepositoryMode,
        storage_mode: StorageMode,
    ) -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let transactions = Arc::new(RecordingTransactions::new(calls.clone()));
        let rate_limiter = Arc::new(RecordingRateLimiter::new(calls.clone(), rate_limit_result));
        let repository = Arc::new(RecordingRepository::new(calls.clone(), repository_mode));
        let object_storage = Arc::new(RecordingObjectStorage::new(calls.clone(), storage_mode));
        let Ok(service) = MediaService::new(
            MediaDependencies {
                transactions,
                repository: repository.clone(),
                object_storage: object_storage.clone(),
                rate_limiter: rate_limiter.clone(),
            },
            MediaEndpointRateLimit {
                limit: 2,
                window: Duration::from_secs(60),
            },
        ) else {
            panic!("test media configuration must be valid");
        };
        Self {
            service,
            calls,
            rate_limiter,
            repository,
            object_storage,
        }
    }

    fn calls(&self) -> Vec<Call> {
        crate::lock_test_mutex(&self.calls, "call").clone()
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

impl RecordingTransactions {
    fn new(calls: Arc<Mutex<Vec<Call>>>) -> Self {
        Self { calls }
    }
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

struct RecordingRateLimiter {
    calls: Arc<Mutex<Vec<Call>>>,
    requests: Mutex<Vec<RateLimitRequest>>,
    result: Result<RateLimitOutcome, RateLimitError>,
}

impl RecordingRateLimiter {
    fn new(calls: Arc<Mutex<Vec<Call>>>, result: Result<RateLimitOutcome, RateLimitError>) -> Self {
        Self {
            calls,
            requests: Mutex::new(Vec::new()),
            result,
        }
    }

    fn requests(&self) -> Vec<RateLimitRequest> {
        crate::lock_test_mutex(&self.requests, "request").clone()
    }
}

impl RateLimiter for RecordingRateLimiter {
    fn check<'a>(&'a self, request: &'a RateLimitRequest) -> RateLimitFuture<'a> {
        record(&self.calls, Call::RateLimit);
        crate::lock_test_mutex(&self.requests, "request").push(request.clone());
        let result = self.result;
        Box::pin(async move { result })
    }
}

#[derive(Clone, Copy)]
enum RepositoryMode {
    Success,
    TargetNotAccessible,
}

struct RecordingRepository {
    calls: Arc<Mutex<Vec<Call>>>,
    commands: Mutex<Vec<CreateUploadIntentCommand>>,
    mode: RepositoryMode,
}

impl RecordingRepository {
    fn new(calls: Arc<Mutex<Vec<Call>>>, mode: RepositoryMode) -> Self {
        Self {
            calls,
            commands: Mutex::new(Vec::new()),
            mode,
        }
    }

    fn commands(&self) -> Vec<CreateUploadIntentCommand> {
        crate::lock_test_mutex(&self.commands, "command").clone()
    }
}

impl MediaRepository for RecordingRepository {
    fn create_upload_intent<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a CreateUploadIntentCommand,
    ) -> MediaRepositoryFuture<'a, UploadIntentRecord> {
        assert!(
            transaction
                .as_any_mut()
                .downcast_mut::<RecordingHandle>()
                .is_some()
        );
        record(&self.calls, Call::Repository);
        crate::lock_test_mutex(&self.commands, "command").push(command.clone());
        let mode = self.mode;
        let command = command.clone();
        Box::pin(async move {
            match mode {
                RepositoryMode::TargetNotAccessible => {
                    return Err(MediaRepositoryError::TargetNotAccessible);
                }
                RepositoryMode::Success => {}
            }
            let created_at = OffsetDateTime::UNIX_EPOCH;
            let expires_in = i64::try_from(command.expires_in.as_secs())
                .map_err(|_| MediaRepositoryError::InvalidData)?;
            let expires_at = created_at + time::Duration::seconds(expires_in);
            Ok(UploadIntentRecord {
                id: command.id,
                user_id: command.user_id,
                scope: command.scope,
                target_id: command.target_id,
                object_key: command.object_key,
                kind: command.kind,
                content_type: command.content_type,
                byte_size: command.byte_size,
                filename: command.filename,
                expires_at,
                created_at,
            })
        })
    }

    fn prepare_upload_finalize<'a>(
        &'a self,
        _query: &'a PrepareUploadFinalizeQuery,
    ) -> MediaRepositoryFuture<'a, UploadFinalizePreparation> {
        Box::pin(async { panic!("upload-intent tests must not prepare finalize") })
    }

    fn finalize_upload<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _command: &'a FinalizeUploadCommand,
    ) -> MediaRepositoryFuture<'a, UploadFinalizeRecord> {
        Box::pin(async { panic!("upload-intent tests must not finalize") })
    }
}

#[derive(Clone, Copy)]
enum StorageMode {
    Success,
    Unavailable,
}

struct RecordingObjectStorage {
    calls: Arc<Mutex<Vec<Call>>>,
    requests: Mutex<Vec<PresignPutRequest>>,
    mode: StorageMode,
}

impl RecordingObjectStorage {
    fn new(calls: Arc<Mutex<Vec<Call>>>, mode: StorageMode) -> Self {
        Self {
            calls,
            requests: Mutex::new(Vec::new()),
            mode,
        }
    }

    fn requests(&self) -> Vec<PresignPutRequest> {
        crate::lock_test_mutex(&self.requests, "presign").clone()
    }
}

impl MediaObjectStorage for RecordingObjectStorage {
    fn presign_put<'a>(
        &'a self,
        request: &'a PresignPutRequest,
    ) -> MediaObjectStorageFuture<'a, PresignedPut> {
        record(&self.calls, Call::Presign);
        crate::lock_test_mutex(&self.requests, "presign").push(request.clone());
        let mode = self.mode;
        let request = request.clone();
        Box::pin(async move {
            match mode {
                StorageMode::Success => Ok(PresignedPut {
                    url: format!("https://media.example.test/{}?signed", request.object_key),
                    expires_in: request.expires_in,
                }),
                StorageMode::Unavailable => Err(ObjectStorageProviderError::Unavailable),
            }
        })
    }

    fn inspect_object<'a>(
        &'a self,
        _request: &'a InspectObjectRequest,
    ) -> MediaObjectStorageFuture<'a, InspectedObject> {
        Box::pin(async { panic!("upload-intent tests must not inspect objects") })
    }
}

fn record(calls: &Mutex<Vec<Call>>, call: Call) {
    crate::lock_test_mutex(calls, "call").push(call);
}

fn actor_id() -> Uuid {
    Uuid::from_u128(0xaaaaaaaa_aaaa_4aaa_8aaa_aaaaaaaaaaaa)
}

fn target_id() -> Uuid {
    Uuid::from_u128(0xbbbbbbbb_bbbb_4bbb_8bbb_bbbbbbbbbbbb)
}

fn input(scope: MediaScope) -> UploadIntentCreateInput {
    UploadIntentCreateInput {
        scope,
        target_id: target_id(),
        content_type: "image/jpeg".to_owned(),
        byte_size: 1_024,
        filename: None,
    }
}
