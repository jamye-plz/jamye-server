use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{Method, StatusCode, header},
    response::Response,
};
use jamye_server::{
    adapters::{
        object_storage::media::S3MediaObjectStorage,
        postgres::{
            media::PostgresMediaRepository, topics::PostgresTopicsRepository,
            transactions::SqlxTransactionManager,
        },
    },
    config::{
        AppEnvironment,
        object_storage::{ObjectStorageConfig, ObjectStorageConfigInput},
    },
    domain::media::{FinalizedObject, InspectedObject, MediaKind, MediaScope},
    ports::{
        media::{
            FinalizeUploadCommand, MediaRepository, MediaRepositoryError,
            PrepareUploadFinalizeQuery, UploadFinalizePreparation, UploadFinalizeRecord,
        },
        object_storage::{InspectObjectRequest, MediaObjectStorage},
        topics::{TopicStatus, TopicsRepository},
        transactions::TransactionManager,
    },
};
use sqlx::PgPool;
use symphonia::core::{checksum::Crc32, io::Monitor};
use time::{Duration as TimeDuration, OffsetDateTime};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

const PRIVATE_BUCKET: &str = "jamye-private-media";
const ACCESS_KEY: &str = "task-8-finalize-access-key";
const SECRET_KEY: &str = "task-8-finalize-secret-key";
const CONTENT_TYPE: &str = "image/jpeg";
const BYTE_SIZE: u64 = 1_024;
const FILENAME: &str = " 여름/기록.jpg ";

#[tokio::test]
async fn postgres_prepare_authorizes_upload_owner_and_topic_manager_before_finalize() -> TestResult
{
    let fixture = FinalizeDatabaseFixture::new().await?;
    let repository = PostgresMediaRepository::new(fixture.pool.clone());

    let chat = fixture
        .insert_upload(
            fixture.author_id,
            MediaScope::Chat,
            fixture.chatroom_id,
            false,
        )
        .await?;
    let prepared = repository
        .prepare_upload_finalize(&query(fixture.author_id, chat.id, None, None))
        .await?;
    let UploadFinalizePreparation::Pending(record) = prepared else {
        return Err(io::Error::other("pending chat upload returned a retry result").into());
    };
    assert_eq!(record.id, chat.id);
    assert_eq!(record.object_key, chat.object_key);
    assert_eq!(record.kind, MediaKind::Image);
    assert_eq!(record.filename.as_deref(), Some(FILENAME));

    assert_eq!(
        repository
            .prepare_upload_finalize(&query(fixture.owner_id, chat.id, None, None))
            .await,
        Err(MediaRepositoryError::TargetNotAccessible)
    );
    assert_eq!(
        repository
            .prepare_upload_finalize(&query(fixture.outsider_id, chat.id, None, None))
            .await,
        Err(MediaRepositoryError::TargetNotAccessible)
    );

    let author_topic = fixture
        .insert_upload(
            fixture.author_id,
            MediaScope::Topic,
            fixture.topic_id,
            false,
        )
        .await?;
    assert!(matches!(
        repository
            .prepare_upload_finalize(&query(
                fixture.author_id,
                author_topic.id,
                Some(800),
                Some(600),
            ))
            .await,
        Ok(UploadFinalizePreparation::Pending(_))
    ));

    let owner_topic = fixture
        .insert_upload(fixture.owner_id, MediaScope::Topic, fixture.topic_id, false)
        .await?;
    assert!(matches!(
        repository
            .prepare_upload_finalize(&query(
                fixture.owner_id,
                owner_topic.id,
                Some(800),
                Some(600),
            ))
            .await,
        Ok(UploadFinalizePreparation::Pending(_))
    ));

    let ordinary_member_topic = fixture
        .insert_upload(
            fixture.member_id,
            MediaScope::Topic,
            fixture.topic_id,
            false,
        )
        .await?;
    assert_eq!(
        repository
            .prepare_upload_finalize(&query(
                fixture.member_id,
                ordinary_member_topic.id,
                Some(800),
                Some(600),
            ))
            .await,
        Err(MediaRepositoryError::TargetNotAccessible)
    );

    let expired = fixture
        .insert_upload(
            fixture.author_id,
            MediaScope::Chat,
            fixture.chatroom_id,
            true,
        )
        .await?;
    assert_eq!(
        repository
            .prepare_upload_finalize(&query(fixture.author_id, expired.id, None, None))
            .await,
        Err(MediaRepositoryError::FinalizeConflict)
    );

    fixture.dispose().await
}

#[tokio::test]
async fn postgres_chat_finalize_obeys_caller_rollback_and_returns_canonical_retry() -> TestResult {
    let fixture = FinalizeDatabaseFixture::new().await?;
    let repository = PostgresMediaRepository::new(fixture.pool.clone());
    let transactions = SqlxTransactionManager::new(fixture.pool.clone());
    let upload = fixture
        .insert_upload(
            fixture.author_id,
            MediaScope::Chat,
            fixture.chatroom_id,
            false,
        )
        .await?;
    let command = FinalizeUploadCommand::Chat {
        actor_id: fixture.author_id,
        upload_id: upload.id,
        finalized: finalized_image(),
    };

    let mut rolled_back_transaction = transactions.begin().await?;
    let rolled_back = repository
        .finalize_upload(rolled_back_transaction.as_mut(), &command)
        .await?;
    assert!(matches!(rolled_back, UploadFinalizeRecord::Chat { .. }));
    transactions.rollback(rolled_back_transaction).await?;
    assert_eq!(
        upload_state(&fixture.pool, upload.id).await?,
        ("pending".to_owned(), None, None, None)
    );

    let mut committed_transaction = transactions.begin().await?;
    let canonical = repository
        .finalize_upload(committed_transaction.as_mut(), &command)
        .await?;
    transactions.commit(committed_transaction).await?;
    let UploadFinalizeRecord::Chat { upload: confirmed } = &canonical else {
        return Err(io::Error::other("chat finalize returned a topic binding").into());
    };
    assert_eq!(confirmed.id, upload.id);
    assert_eq!(confirmed.user_id, fixture.author_id);
    assert_eq!(confirmed.target_id, fixture.chatroom_id);
    assert_eq!(confirmed.content_type, CONTENT_TYPE);
    assert_eq!(confirmed.byte_size, BYTE_SIZE);
    assert_eq!(confirmed.duration_seconds, None);
    assert_eq!(confirmed.filename.as_deref(), Some(FILENAME));

    let retry = repository
        .prepare_upload_finalize(&query(fixture.author_id, upload.id, None, None))
        .await?;
    assert_eq!(retry, UploadFinalizePreparation::Existing(canonical));
    let state = upload_state(&fixture.pool, upload.id).await?;
    assert_eq!(state.0, "confirmed");
    assert!(state.1.is_some());
    assert_eq!(state.2, None);
    assert_eq!(state.3, None);

    fixture.dispose().await
}

#[tokio::test]
async fn postgres_topic_finalize_binding_and_promotion_share_one_atomic_handle() -> TestResult {
    let fixture = FinalizeDatabaseFixture::new().await?;
    let repository = PostgresMediaRepository::new(fixture.pool.clone());
    let topics = PostgresTopicsRepository::new(fixture.pool.clone());
    let transactions = SqlxTransactionManager::new(fixture.pool.clone());
    let upload = fixture
        .insert_upload(
            fixture.author_id,
            MediaScope::Topic,
            fixture.topic_id,
            false,
        )
        .await?;
    let topic_media_id = Uuid::new_v4();
    let command = FinalizeUploadCommand::Topic {
        actor_id: fixture.author_id,
        upload_id: upload.id,
        topic_media_id,
        width: Some(800),
        height: Some(600),
        finalized: finalized_image(),
    };

    let mut rolled_back_transaction = transactions.begin().await?;
    let rolled_back = repository
        .finalize_upload(rolled_back_transaction.as_mut(), &command)
        .await?;
    assert!(matches!(rolled_back, UploadFinalizeRecord::Topic { .. }));
    assert_eq!(
        topics
            .promote_enriched(rolled_back_transaction.as_mut(), fixture.topic_id)
            .await?,
        TopicStatus::Enriched
    );
    transactions.rollback(rolled_back_transaction).await?;
    assert_eq!(
        upload_state(&fixture.pool, upload.id).await?,
        ("pending".to_owned(), None, None, None)
    );
    assert_eq!(topic_media_count(&fixture.pool, upload.id).await?, 0);
    assert_eq!(topic_status(&fixture.pool, fixture.topic_id).await?, "seed");

    let mut committed_transaction = transactions.begin().await?;
    let canonical = repository
        .finalize_upload(committed_transaction.as_mut(), &command)
        .await?;
    assert_eq!(
        topics
            .promote_enriched(committed_transaction.as_mut(), fixture.topic_id)
            .await?,
        TopicStatus::Enriched
    );
    transactions.commit(committed_transaction).await?;

    let UploadFinalizeRecord::Topic {
        upload: confirmed,
        topic_media,
    } = &canonical
    else {
        return Err(io::Error::other("topic finalize returned an unbound chat upload").into());
    };
    assert_eq!(confirmed.id, upload.id);
    assert_eq!(confirmed.target_id, fixture.topic_id);
    assert_eq!(topic_media.id, topic_media_id);
    assert_eq!(topic_media.topic_id, fixture.topic_id);
    assert_eq!(topic_media.media_upload_id, upload.id);
    assert_eq!(topic_media.width, Some(800));
    assert_eq!(topic_media.height, Some(600));
    assert_eq!(topic_media.byte_size, BYTE_SIZE);

    assert_eq!(
        repository
            .prepare_upload_finalize(&query(fixture.author_id, upload.id, Some(800), Some(600),))
            .await?,
        UploadFinalizePreparation::Existing(canonical)
    );
    assert_eq!(
        repository
            .prepare_upload_finalize(&query(fixture.author_id, upload.id, Some(801), Some(600),))
            .await,
        Err(MediaRepositoryError::FinalizeConflict)
    );
    let state = upload_state(&fixture.pool, upload.id).await?;
    assert_eq!(state.0, "bound");
    assert!(state.1.is_some());
    assert_eq!(state.2, Some(topic_media_id));
    assert!(state.3.is_some());
    assert_eq!(topic_media_count(&fixture.pool, upload.id).await?, 1);
    assert_eq!(
        topic_status(&fixture.pool, fixture.topic_id).await?,
        "enriched"
    );

    fixture.dispose().await
}

#[tokio::test]
async fn sdk_head_object_uses_internal_signed_path_and_authoritative_metadata() -> TestResult {
    let server = ScriptedS3::start([ScriptedResponse::head(
        StatusCode::OK,
        Some(CONTENT_TYPE),
        Some(BYTE_SIZE),
    )])
    .await?;
    let storage = object_storage(server.endpoint())?;
    let object_key = object_key(MediaScope::Chat, Uuid::new_v4(), Uuid::new_v4());

    let inspected = storage
        .inspect_object(&InspectObjectRequest {
            object_key: object_key.clone(),
            kind: MediaKind::Image,
        })
        .await?;
    assert_eq!(
        inspected,
        InspectedObject {
            content_type: Some(CONTENT_TYPE.to_owned()),
            byte_size: Some(BYTE_SIZE),
            audio_duration: None,
        }
    );
    drop(storage);
    assert_eq!(
        server.finish().await?,
        vec![ExpectedRequest::object(Method::HEAD, &object_key)]
    );
    Ok(())
}

#[tokio::test]
async fn sdk_audio_duration_uses_header_then_packet_fallback_without_decoding() -> TestResult {
    for is_last_page in [true, false] {
        let audio = ogg_opus_fixture(is_last_page);
        let byte_size = u64::try_from(audio.len())?;
        let server = ScriptedS3::start([
            ScriptedResponse::head(StatusCode::OK, Some("audio/ogg"), Some(byte_size)),
            ScriptedResponse::body(StatusCode::OK, "audio/ogg", audio),
        ])
        .await?;
        let storage = object_storage(server.endpoint())?;
        let object_key = object_key(MediaScope::Chat, Uuid::new_v4(), Uuid::new_v4());

        let inspected = storage
            .inspect_object(&InspectObjectRequest {
                object_key: object_key.clone(),
                kind: MediaKind::Audio,
            })
            .await?;
        assert_eq!(inspected.content_type.as_deref(), Some("audio/ogg"));
        assert_eq!(inspected.byte_size, Some(byte_size));
        assert_eq!(inspected.audio_duration, Some(Duration::from_millis(10)));
        drop(storage);
        assert_eq!(
            server.finish().await?,
            vec![
                ExpectedRequest::object(Method::HEAD, &object_key),
                ExpectedRequest::object(Method::GET, &object_key),
            ]
        );
    }
    Ok(())
}

fn query(
    actor_id: Uuid,
    upload_id: Uuid,
    width: Option<u32>,
    height: Option<u32>,
) -> PrepareUploadFinalizeQuery {
    PrepareUploadFinalizeQuery {
        actor_id,
        upload_id,
        width,
        height,
    }
}

fn finalized_image() -> FinalizedObject {
    FinalizedObject {
        kind: MediaKind::Image,
        content_type: CONTENT_TYPE.to_owned(),
        byte_size: BYTE_SIZE,
        duration_seconds: None,
    }
}

#[derive(Clone, Debug)]
struct SeededUpload {
    id: Uuid,
    object_key: String,
}

struct FinalizeDatabaseFixture {
    database: TestDatabase,
    pool: PgPool,
    owner_id: Uuid,
    author_id: Uuid,
    member_id: Uuid,
    outsider_id: Uuid,
    chatroom_id: Uuid,
    topic_id: Uuid,
}

impl FinalizeDatabaseFixture {
    async fn new() -> TestResult<Self> {
        let database = TestDatabase::migrated().await?;
        let pool = database.pool()?;
        let owner_id = insert_user(&pool, "미디어 그룹 소유자").await?;
        let author_id = insert_user(&pool, "미디어 주제 작성자").await?;
        let member_id = insert_user(&pool, "미디어 일반 멤버").await?;
        let outsider_id = insert_user(&pool, "미디어 외부인").await?;
        let group_id = Uuid::new_v4();
        sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, '미디어 그룹', $2)")
            .bind(group_id)
            .bind(owner_id)
            .execute(&pool)
            .await?;
        for (user_id, role) in [
            (owner_id, "owner"),
            (author_id, "member"),
            (member_id, "member"),
        ] {
            sqlx::query(
                "INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, $4)",
            )
            .bind(Uuid::new_v4())
            .bind(group_id)
            .bind(user_id)
            .bind(role)
            .execute(&pool)
            .await?;
        }
        let chatroom_id = Uuid::new_v4();
        sqlx::query("INSERT INTO chatrooms (id, group_id, type) VALUES ($1, $2, 'main')")
            .bind(chatroom_id)
            .bind(group_id)
            .execute(&pool)
            .await?;
        let topic_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO topics \
                 (id, group_id, author_id, idempotency_key, request_fingerprint, title) \
             VALUES ($1, $2, $3, $4, $5, '미디어 주제')",
        )
        .bind(topic_id)
        .bind(group_id)
        .bind(author_id)
        .bind(Uuid::new_v4())
        .bind("b".repeat(64))
        .execute(&pool)
        .await?;

        Ok(Self {
            database,
            pool,
            owner_id,
            author_id,
            member_id,
            outsider_id,
            chatroom_id,
            topic_id,
        })
    }

    async fn insert_upload(
        &self,
        user_id: Uuid,
        scope: MediaScope,
        target_id: Uuid,
        expired: bool,
    ) -> TestResult<SeededUpload> {
        let id = Uuid::new_v4();
        let object_key = object_key(scope, target_id, id);
        let now = OffsetDateTime::now_utc();
        let (created_at, expires_at) = if expired {
            (now - TimeDuration::hours(2), now - TimeDuration::hours(1))
        } else {
            (now, now + TimeDuration::hours(1))
        };
        let scope = match scope {
            MediaScope::Chat => "chat",
            MediaScope::Topic => "topic",
        };
        sqlx::query(
            "INSERT INTO media_uploads \
                 (id, user_id, object_key, scope, target_id, content_type, byte_size, filename, \
                  expires_at, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(id)
        .bind(user_id)
        .bind(&object_key)
        .bind(scope)
        .bind(target_id)
        .bind(CONTENT_TYPE)
        .bind(i64::try_from(BYTE_SIZE)?)
        .bind(FILENAME)
        .bind(expires_at)
        .bind(created_at)
        .execute(&self.pool)
        .await?;
        Ok(SeededUpload { id, object_key })
    }

    async fn dispose(self) -> TestResult {
        let Self { database, pool, .. } = self;
        pool.close().await;
        database.dispose().await
    }
}

async fn insert_user(pool: &PgPool, nickname: &str) -> TestResult<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, $2)")
        .bind(id)
        .bind(nickname)
        .execute(pool)
        .await?;
    Ok(id)
}

async fn upload_state(
    pool: &PgPool,
    upload_id: Uuid,
) -> TestResult<(
    String,
    Option<OffsetDateTime>,
    Option<Uuid>,
    Option<OffsetDateTime>,
)> {
    Ok(sqlx::query_as(
        "SELECT status, confirmed_at, bound_topic_media_id, consumed_at \
         FROM media_uploads WHERE id = $1",
    )
    .bind(upload_id)
    .fetch_one(pool)
    .await?)
}

async fn topic_media_count(pool: &PgPool, upload_id: Uuid) -> TestResult<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM topic_media WHERE media_upload_id = $1")
            .bind(upload_id)
            .fetch_one(pool)
            .await?,
    )
}

async fn topic_status(pool: &PgPool, topic_id: Uuid) -> TestResult<String> {
    Ok(
        sqlx::query_scalar("SELECT status FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_one(pool)
            .await?,
    )
}

fn object_key(scope: MediaScope, target_id: Uuid, upload_id: Uuid) -> String {
    let prefix = match scope {
        MediaScope::Chat => "chat",
        MediaScope::Topic => "topics",
    };
    format!("{prefix}/{target_id}/{upload_id}")
}

fn object_storage(endpoint: &str) -> TestResult<S3MediaObjectStorage> {
    let config = ObjectStorageConfig::resolve(
        AppEnvironment::Test,
        ObjectStorageConfigInput {
            endpoint: Some(endpoint.to_owned()),
            public_endpoint: Some("http://127.0.0.1:9".to_owned()),
            region: Some("us-east-1".to_owned()),
            bucket: Some(PRIVATE_BUCKET.to_owned()),
            access_key_id: Some(ACCESS_KEY.to_owned()),
            secret_access_key: Some(SECRET_KEY.to_owned()),
        },
    )?
    .ok_or_else(|| io::Error::other("complete test object-storage config resolved absent"))?;
    Ok(S3MediaObjectStorage::new(&config))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExpectedRequest {
    method: Method,
    path: String,
    signed: bool,
    secret_exposed: bool,
}

impl ExpectedRequest {
    fn object(method: Method, object_key: &str) -> Self {
        Self {
            method,
            path: format!("/{PRIVATE_BUCKET}/{object_key}"),
            signed: true,
            secret_exposed: false,
        }
    }
}

#[derive(Clone, Debug)]
struct ScriptedResponse {
    status: StatusCode,
    content_type: Option<String>,
    content_length: Option<u64>,
    body: Vec<u8>,
}

impl ScriptedResponse {
    fn head(status: StatusCode, content_type: Option<&str>, content_length: Option<u64>) -> Self {
        Self {
            status,
            content_type: content_type.map(ToOwned::to_owned),
            content_length,
            body: Vec::new(),
        }
    }

    fn body(status: StatusCode, content_type: &str, body: Vec<u8>) -> Self {
        Self {
            status,
            content_type: Some(content_type.to_owned()),
            content_length: u64::try_from(body.len()).ok(),
            body,
        }
    }
}

#[derive(Clone)]
struct ScriptState {
    responses: Arc<Mutex<VecDeque<ScriptedResponse>>>,
    requests: Arc<Mutex<Vec<ExpectedRequest>>>,
}

struct ScriptedS3 {
    endpoint: String,
    state: ScriptState,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<Result<(), io::Error>>,
}

impl ScriptedS3 {
    async fn start(responses: impl IntoIterator<Item = ScriptedResponse>) -> TestResult<Self> {
        let state = ScriptState {
            responses: Arc::new(Mutex::new(responses.into_iter().collect())),
            requests: Arc::new(Mutex::new(Vec::new())),
        };
        let app = Router::new()
            .fallback(handle_request)
            .with_state(state.clone());
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let (shutdown, shutdown_receiver) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_receiver.await;
                })
                .await
        });

        Ok(Self {
            endpoint: format!("http://{address}"),
            state,
            shutdown,
            task,
        })
    }

    fn endpoint(&self) -> &str {
        &self.endpoint
    }

    async fn finish(self) -> TestResult<Vec<ExpectedRequest>> {
        self.shutdown
            .send(())
            .map_err(|_| io::Error::other("scripted S3 server stopped before shutdown"))?;
        self.task.await??;
        let remaining = self
            .state
            .responses
            .lock()
            .map_err(|_| io::Error::other("scripted S3 response mutex is poisoned"))?;
        if !remaining.is_empty() {
            return Err(io::Error::other("scripted S3 did not receive every request").into());
        }
        drop(remaining);
        self.state
            .requests
            .lock()
            .map(|requests| requests.clone())
            .map_err(|_| io::Error::other("scripted S3 request mutex is poisoned").into())
    }
}

async fn handle_request(
    State(state): State<ScriptState>,
    request: Request<Body>,
) -> Response<Body> {
    let authorization = request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    let observed = ExpectedRequest {
        method: request.method().clone(),
        path: request.uri().path().to_owned(),
        signed: authorization.is_some_and(|value| value.starts_with("AWS4-HMAC-SHA256 ")),
        secret_exposed: authorization.is_some_and(|value| value.contains(SECRET_KEY)),
    };
    let recorded = state
        .requests
        .lock()
        .map(|mut requests| requests.push(observed))
        .is_ok();
    let response = state
        .responses
        .lock()
        .ok()
        .and_then(|mut responses| responses.pop_front());
    let Some(response) = response.filter(|_| recorded) else {
        return Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::empty())
            .unwrap_or_else(|_| Response::new(Body::empty()));
    };

    let mut builder = Response::builder().status(response.status);
    if let Some(content_type) = response.content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    if let Some(content_length) = response.content_length {
        builder = builder.header(header::CONTENT_LENGTH, content_length.to_string());
    }
    builder
        .body(Body::from(response.body))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn ogg_opus_fixture(is_last_page: bool) -> Vec<u8> {
    let serial = 0x4a41_4d59;
    let mut opus_head = Vec::from(&b"OpusHead"[..]);
    opus_head.extend([1, 1]);
    opus_head.extend(0_u16.to_le_bytes());
    opus_head.extend(48_000_u32.to_le_bytes());
    opus_head.extend(0_i16.to_le_bytes());
    opus_head.push(0);

    let mut opus_tags = Vec::from(&b"OpusTags"[..]);
    opus_tags.extend(0_u32.to_le_bytes());
    opus_tags.extend(0_u32.to_le_bytes());

    let mut fixture = ogg_page(serial, 0, 0x02, 0, &opus_head);
    fixture.extend(ogg_page(serial, 1, 0, 0, &opus_tags));
    fixture.extend(ogg_page(
        serial,
        2,
        u8::from(is_last_page) * 0x04,
        480,
        &[0],
    ));
    fixture
}

fn ogg_page(serial: u32, sequence: u32, flags: u8, granule: u64, packet: &[u8]) -> Vec<u8> {
    assert!(!packet.is_empty() && packet.len() < 255);
    let mut page = Vec::with_capacity(28 + packet.len());
    page.extend(b"OggS");
    page.push(0);
    page.push(flags);
    page.extend(granule.to_le_bytes());
    page.extend(serial.to_le_bytes());
    page.extend(sequence.to_le_bytes());
    page.extend(0_u32.to_le_bytes());
    page.push(1);
    page.push(packet.len() as u8);
    page.extend(packet);

    let mut crc = Crc32::new(0);
    crc.process_buf_bytes(&page);
    page[22..26].copy_from_slice(&crc.crc().to_le_bytes());
    page
}
