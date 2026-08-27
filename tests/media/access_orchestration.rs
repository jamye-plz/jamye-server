use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use jamye_server::{
    application::media::{MediaAccessDependencies, MediaAccessService, MediaAccessUrl, MediaError},
    domain::media::{InspectedObject, PRESIGNED_GET_TTL_SECONDS},
    ports::{
        media::{
            AuthorizeMediaAccessQuery, CreateUploadIntentCommand, FinalizeUploadCommand,
            MediaAccessRecord, MediaRepository, MediaRepositoryError, MediaRepositoryFuture,
            PrepareUploadFinalizeQuery, UploadFinalizePreparation, UploadFinalizeRecord,
            UploadIntentRecord,
        },
        object_storage::{
            InspectObjectRequest, MediaObjectStorage, MediaObjectStorageFuture,
            ObjectStorageProviderError, PresignGetRequest, PresignPutRequest, PresignedGet,
            PresignedPut,
        },
        transactions::TransactionHandle,
    },
};
use uuid::Uuid;

#[tokio::test]
async fn authorization_failure_stops_before_view_or_download_presign() {
    let view = Harness::new(RepositoryMode::TargetNotAccessible, StorageMode::Success);
    assert_eq!(
        view.service.view_url(actor_id(), media_id()).await,
        Err(MediaError::TargetNotAccessible)
    );
    assert_eq!(view.calls(), vec![Call::Authorize]);

    let download = Harness::new(RepositoryMode::TargetNotAccessible, StorageMode::Success);
    assert_eq!(
        download.service.download_url(actor_id(), media_id()).await,
        Err(MediaError::TargetNotAccessible)
    );
    assert_eq!(download.calls(), vec![Call::Authorize]);
}

#[tokio::test]
async fn view_authorizes_then_presigns_the_db_owned_key_without_a_download_override() {
    let harness = Harness::new(RepositoryMode::Success, StorageMode::Success);

    assert_eq!(
        harness.service.view_url(actor_id(), media_id()).await,
        Ok(expected_view())
    );
    assert_eq!(harness.calls(), vec![Call::Authorize, Call::Presign]);
    assert_eq!(
        harness.repository.queries(),
        vec![AuthorizeMediaAccessQuery {
            actor_id: actor_id(),
            media_id: media_id(),
        }]
    );
    assert_eq!(
        harness.object_storage.requests(),
        vec![PresignGetRequest {
            object_key: object_key(),
            response_content_disposition: None,
            expires_in: Duration::from_secs(PRESIGNED_GET_TTL_SECONDS),
        }]
    );
}

#[tokio::test]
async fn download_authorizes_then_binds_safe_disposition_to_the_short_presign() {
    let harness = Harness::new(RepositoryMode::Success, StorageMode::Success);

    assert_eq!(
        harness.service.download_url(actor_id(), media_id()).await,
        Ok(expected_get())
    );
    assert_eq!(harness.calls(), vec![Call::Authorize, Call::Presign]);
    assert_eq!(
        harness.object_storage.requests(),
        vec![PresignGetRequest {
            object_key: object_key(),
            response_content_disposition: Some(
                concat!(
                    "attachment; filename=\"jamye-11111111-1111-4111-8111-111111111111.jpg\"; ",
                    "filename*=UTF-8''%EC%97%AC%EB%A6%84%20%EA%B8%B0%EB%A1%9D%20",
                    "%28%EC%B5%9C%EC%A2%85%29.jpg"
                )
                .to_owned()
            ),
            expires_in: Duration::from_secs(PRESIGNED_GET_TTL_SECONDS),
        }]
    );
}

#[tokio::test]
async fn storage_failure_is_degraded_only_after_authorization() {
    let harness = Harness::new(RepositoryMode::Success, StorageMode::Unavailable);

    assert_eq!(
        harness.service.view_url(actor_id(), media_id()).await,
        Err(MediaError::ObjectStorageDegraded)
    );
    assert_eq!(harness.calls(), vec![Call::Authorize, Call::Presign]);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Call {
    Authorize,
    Presign,
}

#[derive(Clone, Copy)]
enum RepositoryMode {
    Success,
    TargetNotAccessible,
}

#[derive(Clone, Copy)]
enum StorageMode {
    Success,
    Unavailable,
}

struct Harness {
    service: MediaAccessService,
    calls: Arc<Mutex<Vec<Call>>>,
    repository: Arc<RecordingRepository>,
    object_storage: Arc<RecordingObjectStorage>,
}

impl Harness {
    fn new(repository_mode: RepositoryMode, storage_mode: StorageMode) -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let repository = Arc::new(RecordingRepository::new(calls.clone(), repository_mode));
        let object_storage = Arc::new(RecordingObjectStorage::new(calls.clone(), storage_mode));
        let service = MediaAccessService::new(MediaAccessDependencies {
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

struct RecordingRepository {
    calls: Arc<Mutex<Vec<Call>>>,
    queries: Mutex<Vec<AuthorizeMediaAccessQuery>>,
    mode: RepositoryMode,
}

impl RecordingRepository {
    fn new(calls: Arc<Mutex<Vec<Call>>>, mode: RepositoryMode) -> Self {
        Self {
            calls,
            queries: Mutex::new(Vec::new()),
            mode,
        }
    }

    fn queries(&self) -> Vec<AuthorizeMediaAccessQuery> {
        crate::lock_test_mutex(&self.queries, "query").clone()
    }
}

impl MediaRepository for RecordingRepository {
    fn create_upload_intent<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _command: &'a CreateUploadIntentCommand,
    ) -> MediaRepositoryFuture<'a, UploadIntentRecord> {
        Box::pin(async { panic!("access tests must not create upload intents") })
    }

    fn prepare_upload_finalize<'a>(
        &'a self,
        _query: &'a PrepareUploadFinalizeQuery,
    ) -> MediaRepositoryFuture<'a, UploadFinalizePreparation> {
        Box::pin(async { panic!("access tests must not prepare finalize") })
    }

    fn finalize_upload<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _command: &'a FinalizeUploadCommand,
    ) -> MediaRepositoryFuture<'a, UploadFinalizeRecord> {
        Box::pin(async { panic!("access tests must not finalize uploads") })
    }

    fn authorize_media_access<'a>(
        &'a self,
        query: &'a AuthorizeMediaAccessQuery,
    ) -> MediaRepositoryFuture<'a, MediaAccessRecord> {
        record(&self.calls, Call::Authorize);
        crate::lock_test_mutex(&self.queries, "query").push(*query);
        let mode = self.mode;
        Box::pin(async move {
            match mode {
                RepositoryMode::Success => Ok(media_record()),
                RepositoryMode::TargetNotAccessible => {
                    Err(MediaRepositoryError::TargetNotAccessible)
                }
            }
        })
    }
}

struct RecordingObjectStorage {
    calls: Arc<Mutex<Vec<Call>>>,
    requests: Mutex<Vec<PresignGetRequest>>,
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

    fn requests(&self) -> Vec<PresignGetRequest> {
        crate::lock_test_mutex(&self.requests, "request").clone()
    }
}

impl MediaObjectStorage for RecordingObjectStorage {
    fn presign_put<'a>(
        &'a self,
        _request: &'a PresignPutRequest,
    ) -> MediaObjectStorageFuture<'a, PresignedPut> {
        Box::pin(async { panic!("access tests must not presign PUT") })
    }

    fn inspect_object<'a>(
        &'a self,
        _request: &'a InspectObjectRequest,
    ) -> MediaObjectStorageFuture<'a, InspectedObject> {
        Box::pin(async { panic!("access tests must not inspect objects") })
    }

    fn presign_get<'a>(
        &'a self,
        request: &'a PresignGetRequest,
    ) -> MediaObjectStorageFuture<'a, PresignedGet> {
        record(&self.calls, Call::Presign);
        crate::lock_test_mutex(&self.requests, "request").push(request.clone());
        let mode = self.mode;
        Box::pin(async move {
            match mode {
                StorageMode::Success => Ok(expected_get()),
                StorageMode::Unavailable => Err(ObjectStorageProviderError::Unavailable),
            }
        })
    }
}

fn record(calls: &Mutex<Vec<Call>>, call: Call) {
    crate::lock_test_mutex(calls, "call").push(call);
}

fn media_record() -> MediaAccessRecord {
    MediaAccessRecord {
        id: media_id(),
        media_upload_id: upload_id(),
        object_key: object_key(),
        content_type: "image/jpeg".to_owned(),
        byte_size: 1_024,
        width: Some(800),
        height: Some(600),
        duration_seconds: None,
        filename: Some("여름 기록 (최종).jpg".to_owned()),
    }
}

fn expected_view() -> MediaAccessUrl {
    MediaAccessUrl {
        id: media_id(),
        media_upload_id: upload_id(),
        url: expected_get().url,
        content_type: "image/jpeg".to_owned(),
        byte_size: 1_024,
        width: Some(800),
        height: Some(600),
        duration_seconds: None,
        filename: Some("여름 기록 (최종).jpg".to_owned()),
        expires_in: Duration::from_secs(PRESIGNED_GET_TTL_SECONDS),
    }
}

fn expected_get() -> PresignedGet {
    PresignedGet {
        url: format!(
            "https://media.example.test/{}/?X-Amz-Signature=test",
            object_key()
        ),
        expires_in: Duration::from_secs(PRESIGNED_GET_TTL_SECONDS),
    }
}

fn actor_id() -> Uuid {
    Uuid::from_u128(0xaaaaaaaa_aaaa_4aaa_8aaa_aaaaaaaaaaaa)
}

fn media_id() -> Uuid {
    Uuid::from_u128(0x11111111_1111_4111_8111_111111111111)
}

fn upload_id() -> Uuid {
    Uuid::from_u128(0x22222222_2222_4222_8222_222222222222)
}

fn target_id() -> Uuid {
    Uuid::from_u128(0x33333333_3333_4333_8333_333333333333)
}

fn object_key() -> String {
    format!("chat/{}/{}", target_id(), upload_id())
}
