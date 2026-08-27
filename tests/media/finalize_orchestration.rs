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
            MediaRepository, MediaRepositoryError, MediaRepositoryFuture,
            PrepareUploadFinalizeQuery, TopicMediaBindingRecord, UploadFinalizePreparation,
            UploadFinalizeRecord, UploadIntentRecord,
        },
        object_storage::{
            InspectObjectRequest, MediaObjectStorage, MediaObjectStorageFuture,
            ObjectStorageProviderError, PresignPutRequest, PresignedPut,
        },
        topics::{
            CreateTopicCommand, CreateTopicOutcome, GetTopicQuery, ListTopicDatesQuery,
            ListTopicMediaQuery, ListTopicTagsQuery, ListTopicsQuery, PatchTopicCommand,
            ReplaceTopicTagsCommand, TopicDatePage, TopicMediaPage, TopicPage, TopicRecord,
            TopicStatus, TopicTagPage, TopicsRepository, TopicsRepositoryError,
            TopicsRepositoryFuture,
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
        MediaScope::Chat,
        PrepareMode::TargetNotAccessible,
        InspectMode::Success,
        FinalizeMode::Success,
        PromoteMode::Success,
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
        MediaScope::Topic,
        PrepareMode::Conflict,
        InspectMode::Success,
        FinalizeMode::Success,
        PromoteMode::Success,
    );
    assert_eq!(
        conflict
            .service
            .finalize_upload(actor_id(), upload_id(), topic_input())
            .await,
        Err(MediaError::FinalizeConflict)
    );
    assert_eq!(conflict.calls(), vec![Call::Prepare]);
}

#[tokio::test]
async fn object_or_metadata_failure_happens_before_the_transaction() {
    let unavailable = Harness::new(
        MediaScope::Chat,
        PrepareMode::Pending,
        InspectMode::Unavailable,
        FinalizeMode::Success,
        PromoteMode::Success,
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
        MediaScope::Chat,
        PrepareMode::Pending,
        InspectMode::ContentTypeMismatch,
        FinalizeMode::Success,
        PromoteMode::Success,
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
        MediaScope::Chat,
        PrepareMode::Pending,
        InspectMode::Success,
        FinalizeMode::Success,
        PromoteMode::Success,
    );

    let result = harness
        .service
        .finalize_upload(actor_id(), upload_id(), UploadFinalizeInput::default())
        .await;
    assert_eq!(result, Ok(application_result(MediaScope::Chat)));
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
        }]
    );
    assert_eq!(
        harness.object_storage.inspections(),
        vec![InspectObjectRequest {
            object_key: object_key(MediaScope::Chat),
            kind: MediaKind::Image,
        }]
    );
    assert!(matches!(
        harness.repository.finalizations().as_slice(),
        [FinalizeUploadCommand::Chat { actor_id: actor, upload_id: upload, finalized }]
            if *actor == actor_id()
                && *upload == upload_id()
                && finalized.content_type == "image/jpeg"
                && finalized.byte_size == 1_024
                && finalized.duration_seconds.is_none()
    ));
    assert!(harness.topics.promotions().is_empty());
}

#[tokio::test]
async fn topic_finalize_binds_then_promotes_and_commits_once() {
    let harness = Harness::new(
        MediaScope::Topic,
        PrepareMode::Pending,
        InspectMode::Success,
        FinalizeMode::Success,
        PromoteMode::Success,
    );

    let result = harness
        .service
        .finalize_upload(actor_id(), upload_id(), topic_input())
        .await;
    assert_eq!(result, Ok(application_result(MediaScope::Topic)));
    assert_eq!(
        harness.calls(),
        vec![
            Call::Prepare,
            Call::Inspect,
            Call::Begin,
            Call::Finalize,
            Call::Promote,
            Call::Commit,
        ]
    );
    assert!(matches!(
        harness.repository.finalizations().as_slice(),
        [FinalizeUploadCommand::Topic { actor_id: actor, upload_id: upload, width, height, finalized, .. }]
            if *actor == actor_id()
                && *upload == upload_id()
                && *width == Some(800)
                && *height == Some(600)
                && finalized.content_type == "image/jpeg"
                && finalized.byte_size == 1_024
    ));
    assert_eq!(harness.topics.promotions(), vec![target_id()]);
}

#[tokio::test]
async fn finalize_or_topic_promotion_failure_rolls_back_without_commit() {
    let conflict = Harness::new(
        MediaScope::Topic,
        PrepareMode::Pending,
        InspectMode::Success,
        FinalizeMode::Conflict,
        PromoteMode::Success,
    );
    assert_eq!(
        conflict
            .service
            .finalize_upload(actor_id(), upload_id(), topic_input())
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

    let promotion_failure = Harness::new(
        MediaScope::Topic,
        PrepareMode::Pending,
        InspectMode::Success,
        FinalizeMode::Success,
        PromoteMode::Unavailable,
    );
    assert_eq!(
        promotion_failure
            .service
            .finalize_upload(actor_id(), upload_id(), topic_input())
            .await,
        Err(MediaError::DatabaseUnavailable)
    );
    assert_eq!(
        promotion_failure.calls(),
        vec![
            Call::Prepare,
            Call::Inspect,
            Call::Begin,
            Call::Finalize,
            Call::Promote,
            Call::Rollback,
        ]
    );
}

#[tokio::test]
async fn exact_retry_returns_the_canonical_result_without_io_or_new_transaction() {
    for scope in [MediaScope::Chat, MediaScope::Topic] {
        let harness = Harness::new(
            scope,
            PrepareMode::Existing,
            InspectMode::Unavailable,
            FinalizeMode::Conflict,
            PromoteMode::Unavailable,
        );
        let input = if scope == MediaScope::Topic {
            topic_input()
        } else {
            UploadFinalizeInput::default()
        };

        assert_eq!(
            harness
                .service
                .finalize_upload(actor_id(), upload_id(), input)
                .await,
            Ok(application_result(scope))
        );
        assert_eq!(harness.calls(), vec![Call::Prepare]);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Call {
    Prepare,
    Inspect,
    Begin,
    Finalize,
    Promote,
    Commit,
    Rollback,
}

#[derive(Clone, Copy)]
enum PrepareMode {
    Pending,
    Existing,
    TargetNotAccessible,
    Conflict,
}

#[derive(Clone, Copy)]
enum InspectMode {
    Success,
    ContentTypeMismatch,
    Unavailable,
}

#[derive(Clone, Copy)]
enum FinalizeMode {
    Success,
    Conflict,
}

#[derive(Clone, Copy)]
enum PromoteMode {
    Success,
    Unavailable,
}

struct Harness {
    service: MediaFinalizeService,
    calls: Arc<Mutex<Vec<Call>>>,
    repository: Arc<RecordingRepository>,
    object_storage: Arc<RecordingObjectStorage>,
    topics: Arc<RecordingTopicsRepository>,
}

impl Harness {
    fn new(
        scope: MediaScope,
        prepare_mode: PrepareMode,
        inspect_mode: InspectMode,
        finalize_mode: FinalizeMode,
        promote_mode: PromoteMode,
    ) -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let transactions = Arc::new(RecordingTransactions::new(calls.clone()));
        let repository = Arc::new(RecordingRepository::new(
            calls.clone(),
            scope,
            prepare_mode,
            finalize_mode,
        ));
        let object_storage = Arc::new(RecordingObjectStorage::new(calls.clone(), inspect_mode));
        let topics = Arc::new(RecordingTopicsRepository::new(calls.clone(), promote_mode));
        let service = MediaFinalizeService::new(MediaFinalizeDependencies {
            transactions,
            repository: repository.clone(),
            object_storage: object_storage.clone(),
            topics: topics.clone(),
        });
        Self {
            service,
            calls,
            repository,
            object_storage,
            topics,
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

struct RecordingRepository {
    calls: Arc<Mutex<Vec<Call>>>,
    scope: MediaScope,
    prepare_mode: PrepareMode,
    finalize_mode: FinalizeMode,
    preparations: Mutex<Vec<PrepareUploadFinalizeQuery>>,
    finalizations: Mutex<Vec<FinalizeUploadCommand>>,
}

impl RecordingRepository {
    fn new(
        calls: Arc<Mutex<Vec<Call>>>,
        scope: MediaScope,
        prepare_mode: PrepareMode,
        finalize_mode: FinalizeMode,
    ) -> Self {
        Self {
            calls,
            scope,
            prepare_mode,
            finalize_mode,
            preparations: Mutex::new(Vec::new()),
            finalizations: Mutex::new(Vec::new()),
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
        let scope = self.scope;
        Box::pin(async move {
            match mode {
                PrepareMode::Pending => {
                    Ok(UploadFinalizePreparation::Pending(pending_upload(scope)))
                }
                PrepareMode::Existing => Ok(UploadFinalizePreparation::Existing(
                    repository_result(scope),
                )),
                PrepareMode::TargetNotAccessible => Err(MediaRepositoryError::TargetNotAccessible),
                PrepareMode::Conflict => Err(MediaRepositoryError::FinalizeConflict),
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
        let scope = self.scope;
        Box::pin(async move {
            match mode {
                FinalizeMode::Success => Ok(repository_result(scope)),
                FinalizeMode::Conflict => Err(MediaRepositoryError::FinalizeConflict),
            }
        })
    }
}

struct RecordingObjectStorage {
    calls: Arc<Mutex<Vec<Call>>>,
    mode: InspectMode,
    inspections: Mutex<Vec<InspectObjectRequest>>,
}

impl RecordingObjectStorage {
    fn new(calls: Arc<Mutex<Vec<Call>>>, mode: InspectMode) -> Self {
        Self {
            calls,
            mode,
            inspections: Mutex::new(Vec::new()),
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
        Box::pin(async { panic!("finalize tests must not presign uploads") })
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

struct RecordingTopicsRepository {
    calls: Arc<Mutex<Vec<Call>>>,
    mode: PromoteMode,
    promotions: Mutex<Vec<Uuid>>,
}

impl RecordingTopicsRepository {
    fn new(calls: Arc<Mutex<Vec<Call>>>, mode: PromoteMode) -> Self {
        Self {
            calls,
            mode,
            promotions: Mutex::new(Vec::new()),
        }
    }

    fn promotions(&self) -> Vec<Uuid> {
        crate::lock_test_mutex(&self.promotions, "promotion").clone()
    }
}

impl TopicsRepository for RecordingTopicsRepository {
    fn create_topic<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _command: &'a CreateTopicCommand,
    ) -> TopicsRepositoryFuture<'a, CreateTopicOutcome> {
        Box::pin(async { panic!("finalize tests must not create topics") })
    }

    fn patch_topic<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _command: &'a PatchTopicCommand,
    ) -> TopicsRepositoryFuture<'a, TopicRecord> {
        Box::pin(async { panic!("finalize tests must not patch topics") })
    }

    fn promote_enriched<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        topic_id: Uuid,
    ) -> TopicsRepositoryFuture<'a, TopicStatus> {
        assert!(
            transaction
                .as_any_mut()
                .downcast_mut::<RecordingHandle>()
                .is_some()
        );
        record(&self.calls, Call::Promote);
        crate::lock_test_mutex(&self.promotions, "promotion").push(topic_id);
        let mode = self.mode;
        Box::pin(async move {
            match mode {
                PromoteMode::Success => Ok(TopicStatus::Enriched),
                PromoteMode::Unavailable => Err(TopicsRepositoryError::Unavailable),
            }
        })
    }

    fn replace_tags<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _command: &'a ReplaceTopicTagsCommand,
    ) -> TopicsRepositoryFuture<'a, TopicTagPage> {
        Box::pin(async { panic!("finalize tests must not replace topic tags") })
    }

    fn list_topics(&self, _query: ListTopicsQuery) -> TopicsRepositoryFuture<'_, TopicPage> {
        Box::pin(async { panic!("finalize tests must not list topics") })
    }

    fn list_topic_dates(
        &self,
        _query: ListTopicDatesQuery,
    ) -> TopicsRepositoryFuture<'_, TopicDatePage> {
        Box::pin(async { panic!("finalize tests must not list topic dates") })
    }

    fn get_topic(&self, _query: GetTopicQuery) -> TopicsRepositoryFuture<'_, TopicRecord> {
        Box::pin(async { panic!("finalize tests must not get topics") })
    }

    fn list_tags(&self, _query: ListTopicTagsQuery) -> TopicsRepositoryFuture<'_, TopicTagPage> {
        Box::pin(async { panic!("finalize tests must not list topic tags") })
    }

    fn list_media(
        &self,
        _query: ListTopicMediaQuery,
    ) -> TopicsRepositoryFuture<'_, TopicMediaPage> {
        Box::pin(async { panic!("finalize tests must not list topic media") })
    }
}

fn pending_upload(scope: MediaScope) -> UploadIntentRecord {
    UploadIntentRecord {
        id: upload_id(),
        user_id: actor_id(),
        scope,
        target_id: target_id(),
        object_key: object_key(scope),
        kind: MediaKind::Image,
        content_type: "image/jpeg".to_owned(),
        byte_size: 1_024,
        filename: Some(" 여름/기록.jpg ".to_owned()),
        expires_at: OffsetDateTime::UNIX_EPOCH + time::Duration::hours(1),
        created_at: OffsetDateTime::UNIX_EPOCH,
    }
}

fn confirmed_upload(scope: MediaScope) -> ConfirmedUploadRecord {
    let upload = pending_upload(scope);
    ConfirmedUploadRecord {
        id: upload.id,
        user_id: upload.user_id,
        scope: upload.scope,
        target_id: upload.target_id,
        object_key: upload.object_key,
        kind: upload.kind,
        content_type: upload.content_type,
        byte_size: upload.byte_size,
        duration_seconds: None,
        filename: upload.filename,
        confirmed_at: OffsetDateTime::UNIX_EPOCH + time::Duration::minutes(1),
    }
}

fn topic_media() -> TopicMediaBindingRecord {
    TopicMediaBindingRecord {
        id: topic_media_id(),
        topic_id: target_id(),
        media_upload_id: upload_id(),
        object_key: object_key(MediaScope::Topic),
        content_type: "image/jpeg".to_owned(),
        width: Some(800),
        height: Some(600),
        byte_size: 1_024,
        created_at: OffsetDateTime::UNIX_EPOCH + time::Duration::minutes(1),
    }
}

fn repository_result(scope: MediaScope) -> UploadFinalizeRecord {
    match scope {
        MediaScope::Chat => UploadFinalizeRecord::Chat {
            upload: confirmed_upload(scope),
        },
        MediaScope::Topic => UploadFinalizeRecord::Topic {
            upload: confirmed_upload(scope),
            topic_media: topic_media(),
        },
    }
}

fn application_result(scope: MediaScope) -> UploadFinalizeResult {
    match repository_result(scope) {
        UploadFinalizeRecord::Chat { upload } => UploadFinalizeResult::Chat { upload },
        UploadFinalizeRecord::Topic {
            upload,
            topic_media,
        } => UploadFinalizeResult::Topic {
            upload,
            topic_media,
            topic_status: TopicStatus::Enriched,
        },
    }
}

fn topic_input() -> UploadFinalizeInput {
    UploadFinalizeInput {
        width: Some(800),
        height: Some(600),
    }
}

fn record(calls: &Mutex<Vec<Call>>, call: Call) {
    crate::lock_test_mutex(calls, "call").push(call);
}

fn actor_id() -> Uuid {
    Uuid::from_u128(0xaaaaaaaa_aaaa_4aaa_8aaa_aaaaaaaaaaaa)
}

fn upload_id() -> Uuid {
    Uuid::from_u128(0xbbbbbbbb_bbbb_4bbb_8bbb_bbbbbbbbbbbb)
}

fn target_id() -> Uuid {
    Uuid::from_u128(0xcccccccc_cccc_4ccc_8ccc_cccccccccccc)
}

fn topic_media_id() -> Uuid {
    Uuid::from_u128(0xdddddddd_dddd_4ddd_8ddd_dddddddddddd)
}

fn object_key(scope: MediaScope) -> String {
    let prefix = match scope {
        MediaScope::Chat => "chat",
        MediaScope::Topic => "topics",
    };
    format!("{prefix}/{}/{}", target_id(), upload_id())
}
