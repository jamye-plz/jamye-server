use std::{
    any::Any,
    sync::{Arc, Mutex},
};

use jamye_server::{
    application::media::{
        MediaError, MediaFinalizeDependencies, MediaFinalizeService, UploadFinalizeInput,
        UploadFinalizeResult,
    },
    domain::media::{InspectedObject, MediaKind, MediaScope},
    ports::{
        media::{
            ConfirmedUploadRecord, CreateUploadIntentCommand, FinalizeUploadCommand,
            MediaRepository, MediaRepositoryError, MediaRepositoryFuture, PosterCandidateRecord,
            PrepareUploadFinalizeQuery, UploadFinalizePreparation, UploadFinalizeRecord,
            UploadIntentRecord,
        },
        object_storage::{
            InspectObjectRequest, MediaObjectStorage, MediaObjectStorageFuture,
            ObjectStorageProviderError, PresignPutRequest, PresignedPut,
        },
        transactions::{
            BoxTransactionHandle, TransactionFuture, TransactionHandle, TransactionManager,
        },
    },
};
use time::OffsetDateTime;
use uuid::Uuid;

#[tokio::test]
async fn authorization_or_conflict_fails_before_object_access_and_transaction() {
    let inaccessible = Harness::new(
        PrepareMode::TargetNotAccessible,
        InspectMode::Success,
        FinalizeMode::Success,
    );

    assert_eq!(
        inaccessible
            .service
            .finalize_upload(actor_id(), upload_id(), UploadFinalizeInput::default())
            .await,
        Err(MediaError::TargetNotAccessible)
    );
    assert_eq!(inaccessible.calls(), vec![Call::Prepare]);

    let conflict = Harness::new(
        PrepareMode::Conflict,
        InspectMode::Success,
        FinalizeMode::Success,
    );
    assert_eq!(
        conflict
            .service
            .finalize_upload(actor_id(), upload_id(), UploadFinalizeInput::default())
            .await,
        Err(MediaError::FinalizeConflict)
    );
    assert_eq!(conflict.calls(), vec![Call::Prepare]);
}

#[tokio::test]
async fn object_or_metadata_failure_happens_before_the_transaction() {
    let unavailable = Harness::new(
        PrepareMode::Pending,
        InspectMode::Unavailable,
        FinalizeMode::Success,
    );
    assert_eq!(
        unavailable
            .service
            .finalize_upload(actor_id(), upload_id(), UploadFinalizeInput::default())
            .await,
        Err(MediaError::ObjectStorageDegraded)
    );
    assert_eq!(unavailable.calls(), vec![Call::Prepare, Call::Inspect]);

    let mismatch = Harness::new(
        PrepareMode::Pending,
        InspectMode::ContentTypeMismatch,
        FinalizeMode::Success,
    );
    assert_eq!(
        mismatch
            .service
            .finalize_upload(actor_id(), upload_id(), UploadFinalizeInput::default())
            .await,
        Err(MediaError::FinalizeValidation)
    );
    assert_eq!(mismatch.calls(), vec![Call::Prepare, Call::Inspect]);
}

#[tokio::test]
async fn chat_finalize_inspects_before_one_transaction_and_returns_unbound() {
    let harness = Harness::new(
        PrepareMode::Pending,
        InspectMode::Success,
        FinalizeMode::Success,
    );

    let result = harness
        .service
        .finalize_upload(actor_id(), upload_id(), UploadFinalizeInput::default())
        .await;
    assert_eq!(result, Ok(application_result()));
    assert_eq!(
        harness.calls(),
        vec![
            Call::Prepare,
            Call::Inspect,
            Call::Begin,
            Call::Finalize,
            Call::Commit,
        ]
    );
    assert_eq!(
        harness.repository.preparations(),
        vec![PrepareUploadFinalizeQuery {
            actor_id: actor_id(),
            upload_id: upload_id(),
            width: None,
            height: None,
            poster_upload_id: None,
        }]
    );
    assert_eq!(
        harness.object_storage.inspections(),
        vec![InspectObjectRequest {
            object_key: object_key(),
            kind: MediaKind::Image,
        }]
    );
    assert!(matches!(
        harness.repository.finalizations().as_slice(),
        [FinalizeUploadCommand::Chat {
            actor_id: actor,
            upload_id: upload,
            finalized,
            poster_upload_id: None,
        }]
            if *actor == actor_id()
                && *upload == upload_id()
                && finalized.content_type == "image/jpeg"
                && finalized.byte_size == 1_024
                && finalized.duration_seconds.is_none()
    ));
}

#[tokio::test]
async fn finalize_failure_rolls_back_without_commit() {
    let conflict = Harness::new(
        PrepareMode::Pending,
        InspectMode::Success,
        FinalizeMode::Conflict,
    );
    assert_eq!(
        conflict
            .service
            .finalize_upload(actor_id(), upload_id(), UploadFinalizeInput::default())
            .await,
        Err(MediaError::FinalizeConflict)
    );
    assert_eq!(
        conflict.calls(),
        vec![
            Call::Prepare,
            Call::Inspect,
            Call::Begin,
            Call::Finalize,
            Call::Rollback,
        ]
    );
}

#[tokio::test]
async fn exact_retry_returns_the_canonical_result_without_io_or_new_transaction() {
    let harness = Harness::new(
        PrepareMode::Existing,
        InspectMode::Unavailable,
        FinalizeMode::Conflict,
    );

    assert_eq!(
        harness
            .service
            .finalize_upload(actor_id(), upload_id(), UploadFinalizeInput::default())
            .await,
        Ok(application_result())
    );
    assert_eq!(harness.calls(), vec![Call::Prepare]);
}

#[tokio::test]
async fn valid_poster_upload_id_is_accepted_and_carried_into_the_finalize_command() {
    let poster = valid_poster_candidate();
    let harness = Harness::new_with_poster(
        poster.clone(),
        InspectMode::VideoSuccess,
        FinalizeMode::Success,
    );

    let result = harness
        .service
        .finalize_upload(
            actor_id(),
            upload_id(),
            UploadFinalizeInput {
                width: None,
                height: None,
                poster_upload_id: Some(poster.id),
            },
        )
        .await;
    assert!(
        result.is_ok(),
        "valid poster finalize unexpectedly failed: {result:?}"
    );
    assert!(matches!(
        harness.repository.finalizations().as_slice(),
        [FinalizeUploadCommand::Chat { poster_upload_id: Some(id), .. }] if *id == poster.id
    ));
}

#[tokio::test]
async fn invalid_poster_upload_id_is_rejected_before_any_object_access_or_transaction() {
    let mut poster = valid_poster_candidate();
    poster.already_linked = true;
    let harness = Harness::new_with_poster(
        poster.clone(),
        InspectMode::VideoSuccess,
        FinalizeMode::Success,
    );

    let result = harness
        .service
        .finalize_upload(
            actor_id(),
            upload_id(),
            UploadFinalizeInput {
                width: None,
                height: None,
                poster_upload_id: Some(poster.id),
            },
        )
        .await;

    assert_eq!(result, Err(MediaError::PosterValidation));
    assert_eq!(harness.calls(), vec![Call::Prepare]);
    assert!(harness.object_storage.inspections().is_empty());
    assert!(harness.repository.finalizations().is_empty());
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Call {
    Prepare,
    Inspect,
    Begin,
    Finalize,
    Commit,
    Rollback,
}

struct Harness {
    service: MediaFinalizeService,
    calls: Arc<Mutex<Vec<Call>>>,
    repository: Arc<RecordingRepository>,
    object_storage: Arc<RecordingObjectStorage>,
}

impl Harness {
    fn new(
        prepare_mode: PrepareMode,
        inspect_mode: InspectMode,
        finalize_mode: FinalizeMode,
    ) -> Self {
        Self::with_poster(prepare_mode, None, inspect_mode, finalize_mode)
    }

    fn new_with_poster(
        poster: PosterCandidateRecord,
        inspect_mode: InspectMode,
        finalize_mode: FinalizeMode,
    ) -> Self {
        Self::with_poster(
            PrepareMode::PendingVideo,
            Some(poster),
            inspect_mode,
            finalize_mode,
        )
    }

    fn with_poster(
        prepare_mode: PrepareMode,
        poster: Option<PosterCandidateRecord>,
        inspect_mode: InspectMode,
        finalize_mode: FinalizeMode,
    ) -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let transactions = Arc::new(RecordingTransactions::new(calls.clone()));
        let repository = Arc::new(RecordingRepository::new(
            calls.clone(),
            prepare_mode,
            finalize_mode,
            poster,
        ));
        let object_storage = Arc::new(RecordingObjectStorage::new(calls.clone(), inspect_mode));
        let service = MediaFinalizeService::new(MediaFinalizeDependencies {
            transactions,
            repository: repository.clone(),
            object_storage: object_storage.clone(),
        });
        Self {
            service,
            calls,
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

#[derive(Clone, Copy)]
enum PrepareMode {
    Pending,
    PendingVideo,
    Existing,
    Conflict,
    TargetNotAccessible,
}

#[derive(Clone, Copy)]
enum FinalizeMode {
    Success,
    Conflict,
}

struct RecordingRepository {
    calls: Arc<Mutex<Vec<Call>>>,
    preparations: Mutex<Vec<PrepareUploadFinalizeQuery>>,
    finalizations: Mutex<Vec<FinalizeUploadCommand>>,
    prepare_mode: PrepareMode,
    finalize_mode: FinalizeMode,
    poster: Option<PosterCandidateRecord>,
}

impl RecordingRepository {
    fn new(
        calls: Arc<Mutex<Vec<Call>>>,
        prepare_mode: PrepareMode,
        finalize_mode: FinalizeMode,
        poster: Option<PosterCandidateRecord>,
    ) -> Self {
        Self {
            calls,
            preparations: Mutex::new(Vec::new()),
            finalizations: Mutex::new(Vec::new()),
            prepare_mode,
            finalize_mode,
            poster,
        }
    }

    fn preparations(&self) -> Vec<PrepareUploadFinalizeQuery> {
        crate::lock_test_mutex(&self.preparations, "preparation").clone()
    }

    fn finalizations(&self) -> Vec<FinalizeUploadCommand> {
        crate::lock_test_mutex(&self.finalizations, "finalization").clone()
    }
}

impl MediaRepository for RecordingRepository {
    fn create_upload_intent<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _command: &'a CreateUploadIntentCommand,
    ) -> MediaRepositoryFuture<'a, UploadIntentRecord> {
        Box::pin(async { panic!("finalize tests must not create upload intents") })
    }

    fn prepare_upload_finalize<'a>(
        &'a self,
        query: &'a PrepareUploadFinalizeQuery,
    ) -> MediaRepositoryFuture<'a, UploadFinalizePreparation> {
        record(&self.calls, Call::Prepare);
        crate::lock_test_mutex(&self.preparations, "preparation").push(*query);
        let mode = self.prepare_mode;
        let poster = self.poster.clone();
        Box::pin(async move {
            match mode {
                PrepareMode::TargetNotAccessible => Err(MediaRepositoryError::TargetNotAccessible),
                PrepareMode::Conflict => Err(MediaRepositoryError::FinalizeConflict),
                PrepareMode::Existing => {
                    Ok(UploadFinalizePreparation::Existing(upload_finalize_record()))
                }
                PrepareMode::Pending => Ok(UploadFinalizePreparation::Pending {
                    upload: upload_intent(MediaKind::Image),
                    poster,
                }),
                PrepareMode::PendingVideo => Ok(UploadFinalizePreparation::Pending {
                    upload: upload_intent(MediaKind::Video),
                    poster,
                }),
            }
        })
    }

    fn finalize_upload<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a FinalizeUploadCommand,
    ) -> MediaRepositoryFuture<'a, UploadFinalizeRecord> {
        assert!(
            transaction
                .as_any_mut()
                .downcast_mut::<RecordingHandle>()
                .is_some()
        );
        record(&self.calls, Call::Finalize);
        crate::lock_test_mutex(&self.finalizations, "finalization").push(command.clone());
        let mode = self.finalize_mode;
        Box::pin(async move {
            match mode {
                FinalizeMode::Success => Ok(upload_finalize_record()),
                FinalizeMode::Conflict => Err(MediaRepositoryError::FinalizeConflict),
            }
        })
    }
}

#[derive(Clone, Copy)]
enum InspectMode {
    Success,
    VideoSuccess,
    ContentTypeMismatch,
    Unavailable,
}

struct RecordingObjectStorage {
    calls: Arc<Mutex<Vec<Call>>>,
    inspections: Mutex<Vec<InspectObjectRequest>>,
    mode: InspectMode,
}

impl RecordingObjectStorage {
    fn new(calls: Arc<Mutex<Vec<Call>>>, mode: InspectMode) -> Self {
        Self {
            calls,
            inspections: Mutex::new(Vec::new()),
            mode,
        }
    }

    fn inspections(&self) -> Vec<InspectObjectRequest> {
        crate::lock_test_mutex(&self.inspections, "inspection").clone()
    }
}

impl MediaObjectStorage for RecordingObjectStorage {
    fn presign_put<'a>(
        &'a self,
        _request: &'a PresignPutRequest,
    ) -> MediaObjectStorageFuture<'a, PresignedPut> {
        Box::pin(async { panic!("finalize tests must not presign upload puts") })
    }

    fn inspect_object<'a>(
        &'a self,
        request: &'a InspectObjectRequest,
    ) -> MediaObjectStorageFuture<'a, InspectedObject> {
        record(&self.calls, Call::Inspect);
        crate::lock_test_mutex(&self.inspections, "inspection").push(request.clone());
        let mode = self.mode;
        Box::pin(async move {
            match mode {
                InspectMode::Success => Ok(InspectedObject {
                    content_type: Some("image/jpeg".to_owned()),
                    byte_size: Some(1_024),
                    audio_duration: None,
                }),
                InspectMode::VideoSuccess => Ok(InspectedObject {
                    content_type: Some("video/mp4".to_owned()),
                    byte_size: Some(2_048),
                    audio_duration: None,
                }),
                InspectMode::ContentTypeMismatch => Ok(InspectedObject {
                    content_type: Some("image/png".to_owned()),
                    byte_size: Some(1_024),
                    audio_duration: None,
                }),
                InspectMode::Unavailable => Err(ObjectStorageProviderError::Unavailable),
            }
        })
    }
}

fn record(calls: &Mutex<Vec<Call>>, call: Call) {
    crate::lock_test_mutex(calls, "call").push(call);
}

fn upload_intent(kind: MediaKind) -> UploadIntentRecord {
    UploadIntentRecord {
        id: upload_id(),
        user_id: actor_id(),
        scope: MediaScope::Chat,
        target_id: target_id(),
        object_key: object_key(),
        kind,
        content_type: match kind {
            MediaKind::Image => "image/jpeg".to_owned(),
            MediaKind::Video => "video/mp4".to_owned(),
            MediaKind::Audio => "audio/ogg".to_owned(),
        },
        byte_size: match kind {
            MediaKind::Image => 1_024,
            MediaKind::Video => 2_048,
            MediaKind::Audio => 512,
        },
        filename: None,
        expires_at: OffsetDateTime::UNIX_EPOCH + time::Duration::hours(1),
        created_at: OffsetDateTime::UNIX_EPOCH,
    }
}

fn upload_finalize_record() -> UploadFinalizeRecord {
    UploadFinalizeRecord::Chat {
        upload: confirmed_upload(),
    }
}

fn application_result() -> UploadFinalizeResult {
    UploadFinalizeResult::Chat {
        upload: confirmed_upload(),
    }
}

fn confirmed_upload() -> ConfirmedUploadRecord {
    ConfirmedUploadRecord {
        id: upload_id(),
        user_id: actor_id(),
        scope: MediaScope::Chat,
        target_id: target_id(),
        object_key: object_key(),
        kind: MediaKind::Image,
        content_type: "image/jpeg".to_owned(),
        byte_size: 1_024,
        duration_seconds: None,
        filename: None,
        confirmed_at: OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(1),
        poster_upload_id: None,
    }
}

fn valid_poster_candidate() -> PosterCandidateRecord {
    PosterCandidateRecord {
        id: poster_id(),
        user_id: actor_id(),
        scope: MediaScope::Chat,
        target_id: target_id(),
        kind: MediaKind::Image,
        content_type: "image/jpeg".to_owned(),
        byte_size: 32_000,
        status_confirmed: true,
        already_linked: false,
        has_own_poster: false,
    }
}

fn actor_id() -> Uuid {
    Uuid::from_u128(0xaaaaaaaa_aaaa_4aaa_8aaa_aaaaaaaaaaaa)
}

fn target_id() -> Uuid {
    Uuid::from_u128(0xbbbbbbbb_bbbb_4bbb_8bbb_bbbbbbbbbbbb)
}

fn upload_id() -> Uuid {
    Uuid::from_u128(0xcccccccc_cccc_4ccc_8ccc_cccccccccccc)
}

fn poster_id() -> Uuid {
    Uuid::from_u128(0xdddddddd_dddd_4ddd_8ddd_dddddddddddd)
}

fn object_key() -> String {
    format!("chat/{}/{}", target_id(), upload_id())
}
