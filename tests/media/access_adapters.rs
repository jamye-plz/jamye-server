use std::{collections::BTreeMap, io, time::Duration};

use jamye_server::{
    adapters::{
        object_storage::media::S3MediaObjectStorage, postgres::media::PostgresMediaRepository,
    },
    config::{
        AppEnvironment,
        object_storage::{ObjectStorageConfig, ObjectStorageConfigInput},
    },
    domain::media::PRESIGNED_GET_TTL_SECONDS,
    ports::{
        media::{
            AuthorizeMediaAccessQuery, MediaAccessRecord, MediaRepository, MediaRepositoryError,
        },
        object_storage::{MediaObjectStorage, PresignGetRequest},
    },
};
use sqlx::PgPool;
use time::{Duration as TimeDuration, OffsetDateTime};
use url::Url;
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

const PRIVATE_BUCKET: &str = "jamye-private-media";
const ACCESS_KEY: &str = "task-8-access-get-key";
const SECRET_KEY: &str = "task-8-access-get-secret";
const PUBLIC_PORT: u16 = 45_679;
const DOWNLOAD_DISPOSITION: &str = concat!(
    "attachment; filename=\"jamye-11111111-1111-4111-8111-111111111111.jpg\"; ",
    "filename*=UTF-8''%EC%97%AC%EB%A6%84%20%EA%B8%B0%EB%A1%9D.jpg"
);

#[tokio::test]
async fn postgres_access_uses_message_or_topic_membership_and_rejects_cross_group_ids() -> TestResult
{
    let fixture = AccessDatabaseFixture::new().await?;
    let repository = PostgresMediaRepository::new(fixture.pool.clone());

    let chat = repository
        .authorize_media_access(&query(fixture.member_id, fixture.chat.id))
        .await;
    let topic = repository
        .authorize_media_access(&query(fixture.member_id, fixture.topic.id))
        .await;
    let denied_chat = repository
        .authorize_media_access(&query(fixture.outsider_id, fixture.chat.id))
        .await;
    let denied_topic = repository
        .authorize_media_access(&query(fixture.outsider_id, fixture.topic.id))
        .await;
    let missing = repository
        .authorize_media_access(&query(fixture.member_id, Uuid::new_v4()))
        .await;

    let expected_chat = fixture.chat.clone();
    let expected_topic = fixture.topic.clone();
    fixture.dispose().await?;

    assert_eq!(chat, Ok(expected_chat));
    assert_eq!(topic, Ok(expected_topic));
    assert_eq!(denied_chat, Err(MediaRepositoryError::TargetNotAccessible));
    assert_eq!(denied_topic, Err(MediaRepositoryError::TargetNotAccessible));
    assert_eq!(missing, Err(MediaRepositoryError::TargetNotAccessible));
    Ok(())
}

#[tokio::test]
async fn postgres_access_authorizes_a_bound_poster_and_denies_it_when_its_video_is_unbound()
-> TestResult {
    let fixture = AccessDatabaseFixture::new().await?;
    let repository = PostgresMediaRepository::new(fixture.pool.clone());

    let (chatroom_id, message_id) =
        insert_poster_chatroom(&fixture.pool, fixture.member_id).await?;
    let poster_id = insert_bound_poster(
        &fixture.pool,
        fixture.member_id,
        chatroom_id,
        message_id,
        true,
    )
    .await?;

    let authorized = repository
        .authorize_media_access(&query(fixture.member_id, poster_id))
        .await?;
    assert_eq!(authorized.id, poster_id);
    assert_eq!(authorized.media_upload_id, poster_id);
    assert_eq!(authorized.width, None);
    assert_eq!(authorized.height, None);
    assert_eq!(authorized.content_type, "image/jpeg");

    let denied_outsider = repository
        .authorize_media_access(&query(fixture.outsider_id, poster_id))
        .await;
    assert_eq!(
        denied_outsider,
        Err(MediaRepositoryError::TargetNotAccessible)
    );

    let (chatroom_id, message_id) =
        insert_poster_chatroom(&fixture.pool, fixture.member_id).await?;
    let orphaned_poster_id = insert_bound_poster(
        &fixture.pool,
        fixture.member_id,
        chatroom_id,
        message_id,
        false,
    )
    .await?;
    let denied_unbound_video = repository
        .authorize_media_access(&query(fixture.member_id, orphaned_poster_id))
        .await;
    assert_eq!(
        denied_unbound_video,
        Err(MediaRepositoryError::TargetNotAccessible)
    );

    fixture.dispose().await?;
    Ok(())
}

#[tokio::test]
async fn sdk_view_get_uses_the_public_path_style_origin_and_exact_short_ttl() -> TestResult {
    let storage = object_storage()?;
    let request = PresignGetRequest {
        object_key: object_key(),
        response_content_disposition: None,
        expires_in: Duration::from_secs(PRESIGNED_GET_TTL_SECONDS),
    };

    let presigned = storage.presign_get(&request).await?;
    assert_eq!(presigned.expires_in, request.expires_in);
    assert_presigned_get(&presigned.url, &request.object_key, None)
}

#[tokio::test]
async fn sdk_download_get_binds_the_safe_content_disposition_without_exposing_secrets() -> TestResult
{
    let storage = object_storage()?;
    let request = PresignGetRequest {
        object_key: object_key(),
        response_content_disposition: Some(DOWNLOAD_DISPOSITION.to_owned()),
        expires_in: Duration::from_secs(PRESIGNED_GET_TTL_SECONDS),
    };

    let presigned = storage.presign_get(&request).await?;
    assert_eq!(presigned.expires_in, request.expires_in);
    assert_presigned_get(
        &presigned.url,
        &request.object_key,
        Some(DOWNLOAD_DISPOSITION),
    )
}

fn assert_presigned_get(value: &str, object_key: &str, disposition: Option<&str>) -> TestResult {
    assert!(!value.contains(SECRET_KEY));
    let url = Url::parse(value)?;
    assert_eq!(url.scheme(), "http");
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    assert_eq!(url.port(), Some(PUBLIC_PORT));
    assert_eq!(url.path(), format!("/{PRIVATE_BUCKET}/{object_key}"));

    let query = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        query.get("X-Amz-Algorithm").map(String::as_str),
        Some("AWS4-HMAC-SHA256")
    );
    assert_eq!(query.get("X-Amz-Expires").map(String::as_str), Some("600"));
    assert!(
        query
            .get("X-Amz-Credential")
            .is_some_and(|value| value.starts_with(ACCESS_KEY))
    );
    let signed_headers = query
        .get("X-Amz-SignedHeaders")
        .ok_or_else(|| io::Error::other("GET presign omitted X-Amz-SignedHeaders"))?;
    assert!(signed_headers.split(';').any(|value| value == "host"));
    let signature = query
        .get("X-Amz-Signature")
        .ok_or_else(|| io::Error::other("GET presign omitted X-Amz-Signature"))?;
    assert_eq!(signature.len(), 64);
    assert!(signature.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(
        query
            .get("response-content-disposition")
            .map(String::as_str),
        disposition
    );
    Ok(())
}

fn query(actor_id: Uuid, media_id: Uuid) -> AuthorizeMediaAccessQuery {
    AuthorizeMediaAccessQuery { actor_id, media_id }
}

fn object_key() -> String {
    "chat/33333333-3333-4333-8333-333333333333/22222222-2222-4222-8222-222222222222".to_owned()
}

fn object_storage() -> TestResult<S3MediaObjectStorage> {
    let config = ObjectStorageConfig::resolve(
        AppEnvironment::Test,
        ObjectStorageConfigInput {
            endpoint: Some("http://127.0.0.1:45678".to_owned()),
            public_endpoint: Some(format!("http://127.0.0.1:{PUBLIC_PORT}")),
            region: Some("us-east-1".to_owned()),
            bucket: Some(PRIVATE_BUCKET.to_owned()),
            access_key_id: Some(ACCESS_KEY.to_owned()),
            secret_access_key: Some(SECRET_KEY.to_owned()),
        },
    )?
    .ok_or_else(|| io::Error::other("complete object-storage config resolved absent"))?;
    Ok(S3MediaObjectStorage::new(&config))
}

struct AccessDatabaseFixture {
    database: TestDatabase,
    pool: PgPool,
    member_id: Uuid,
    outsider_id: Uuid,
    chat: MediaAccessRecord,
    topic: MediaAccessRecord,
}

impl AccessDatabaseFixture {
    async fn new() -> TestResult<Self> {
        let database = TestDatabase::migrated().await?;
        let pool = database.pool()?;
        let member_id = insert_user(&pool, "미디어 접근 멤버").await?;
        let outsider_id = insert_user(&pool, "다른 그룹 멤버").await?;

        let group_id = Uuid::new_v4();
        sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, '접근 그룹', $2)")
            .bind(group_id)
            .bind(member_id)
            .execute(&pool)
            .await?;
        sqlx::query(
            "INSERT INTO memberships (id, group_id, user_id, role) \
             VALUES ($1, $2, $3, 'owner')",
        )
        .bind(Uuid::new_v4())
        .bind(group_id)
        .bind(member_id)
        .execute(&pool)
        .await?;

        let outsider_group_id = Uuid::new_v4();
        sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, '외부 접근 그룹', $2)")
            .bind(outsider_group_id)
            .bind(outsider_id)
            .execute(&pool)
            .await?;
        sqlx::query(
            "INSERT INTO memberships (id, group_id, user_id, role) \
             VALUES ($1, $2, $3, 'owner')",
        )
        .bind(Uuid::new_v4())
        .bind(outsider_group_id)
        .bind(outsider_id)
        .execute(&pool)
        .await?;

        let chatroom_id = Uuid::new_v4();
        sqlx::query("INSERT INTO chatrooms (id, group_id, type) VALUES ($1, $2, 'main')")
            .bind(chatroom_id)
            .bind(group_id)
            .execute(&pool)
            .await?;
        let message_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO messages \
                 (id, chatroom_id, sender_id, client_msg_id, body, type) \
             VALUES ($1, $2, $3, $4, '접근 첨부', 'user')",
        )
        .bind(message_id)
        .bind(chatroom_id)
        .bind(member_id)
        .bind(Uuid::new_v4())
        .execute(&pool)
        .await?;
        let chat = insert_chat_media(&pool, member_id, chatroom_id, message_id).await?;

        let topic_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO topics \
                 (id, group_id, author_id, idempotency_key, request_fingerprint, title, status) \
             VALUES ($1, $2, $3, $4, $5, '접근 주제', 'enriched')",
        )
        .bind(topic_id)
        .bind(group_id)
        .bind(member_id)
        .bind(Uuid::new_v4())
        .bind("c".repeat(64))
        .execute(&pool)
        .await?;
        let topic = insert_topic_media(&pool, member_id, topic_id).await?;

        Ok(Self {
            database,
            pool,
            member_id,
            outsider_id,
            chat,
            topic,
        })
    }

    async fn dispose(self) -> TestResult {
        let Self { database, pool, .. } = self;
        pool.close().await;
        database.dispose().await
    }
}

async fn insert_chat_media(
    pool: &PgPool,
    user_id: Uuid,
    chatroom_id: Uuid,
    message_id: Uuid,
) -> TestResult<MediaAccessRecord> {
    let upload_id = Uuid::new_v4();
    let media_id = Uuid::new_v4();
    let object_key = format!("chat/{chatroom_id}/{upload_id}");
    let now = OffsetDateTime::now_utc();
    let confirmed_at = now - TimeDuration::minutes(2);
    let consumed_at = now - TimeDuration::minutes(1);
    let created_at = now - TimeDuration::minutes(3);
    sqlx::query(
        "INSERT INTO media_uploads \
             (id, user_id, object_key, scope, target_id, content_type, byte_size, filename, \
              status, bound_message_id, confirmed_at, consumed_at, expires_at, created_at) \
         VALUES ($1, $2, $3, 'chat', $4, 'image/jpeg', 1024, $5, \
                 'bound', $6, $7, $8, $9, $10)",
    )
    .bind(upload_id)
    .bind(user_id)
    .bind(&object_key)
    .bind(chatroom_id)
    .bind(" 여름/기록.jpg ")
    .bind(message_id)
    .bind(confirmed_at)
    .bind(consumed_at)
    .bind(now + TimeDuration::hours(1))
    .bind(created_at)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO message_media \
             (id, message_id, media_upload_id, type, object_key, width, height, byte_size, \
              position, filename, created_at) \
         VALUES ($1, $2, $3, 'image/jpeg', $4, 800, 600, 1024, 0, $5, $6)",
    )
    .bind(media_id)
    .bind(message_id)
    .bind(upload_id)
    .bind(&object_key)
    .bind(" 여름/기록.jpg ")
    .bind(consumed_at)
    .execute(pool)
    .await?;

    Ok(MediaAccessRecord {
        id: media_id,
        media_upload_id: upload_id,
        object_key,
        content_type: "image/jpeg".to_owned(),
        byte_size: 1_024,
        width: Some(800),
        height: Some(600),
        duration_seconds: None,
        filename: Some(" 여름/기록.jpg ".to_owned()),
    })
}

async fn insert_topic_media(
    pool: &PgPool,
    user_id: Uuid,
    topic_id: Uuid,
) -> TestResult<MediaAccessRecord> {
    let upload_id = Uuid::new_v4();
    let media_id = Uuid::new_v4();
    let object_key = format!("topics/{topic_id}/{upload_id}");
    let now = OffsetDateTime::now_utc();
    let confirmed_at = now - TimeDuration::minutes(2);
    let consumed_at = now - TimeDuration::minutes(1);
    let created_at = now - TimeDuration::minutes(3);
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "INSERT INTO media_uploads \
             (id, user_id, object_key, scope, target_id, content_type, byte_size, filename, \
              status, bound_topic_media_id, confirmed_at, consumed_at, expires_at, created_at) \
         VALUES ($1, $2, $3, 'topic', $4, 'image/png', 2048, $5, \
                 'bound', $6, $7, $8, $9, $10)",
    )
    .bind(upload_id)
    .bind(user_id)
    .bind(&object_key)
    .bind(topic_id)
    .bind("주제 사진.png")
    .bind(media_id)
    .bind(confirmed_at)
    .bind(consumed_at)
    .bind(now + TimeDuration::hours(1))
    .bind(created_at)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO topic_media \
             (id, topic_id, media_upload_id, type, object_key, width, height, byte_size, \
              created_at) \
         VALUES ($1, $2, $3, 'image/png', $4, 640, 480, 2048, $5)",
    )
    .bind(media_id)
    .bind(topic_id)
    .bind(upload_id)
    .bind(&object_key)
    .bind(consumed_at)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    Ok(MediaAccessRecord {
        id: media_id,
        media_upload_id: upload_id,
        object_key,
        content_type: "image/png".to_owned(),
        byte_size: 2_048,
        width: Some(640),
        height: Some(480),
        duration_seconds: None,
        filename: Some("주제 사진.png".to_owned()),
    })
}

/// Create a fresh group/chatroom/message owned by `user_id`, isolated from
/// any other fixture state, and return `(chatroom_id, message_id)`.
async fn insert_poster_chatroom(pool: &PgPool, user_id: Uuid) -> TestResult<(Uuid, Uuid)> {
    let group_id = Uuid::new_v4();
    sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, '포스터 그룹', $2)")
        .bind(group_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, 'owner')",
    )
    .bind(Uuid::new_v4())
    .bind(group_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    let chatroom_id = Uuid::new_v4();
    sqlx::query("INSERT INTO chatrooms (id, group_id, type) VALUES ($1, $2, 'main')")
        .bind(chatroom_id)
        .bind(group_id)
        .execute(pool)
        .await?;
    let message_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO messages (id, chatroom_id, sender_id, client_msg_id, body, type) \
         VALUES ($1, $2, $3, $4, '포스터 첨부', 'user')",
    )
    .bind(message_id)
    .bind(chatroom_id)
    .bind(user_id)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await?;
    Ok((chatroom_id, message_id))
}

/// Seed a poster upload bound to `message_id`, plus a video upload whose
/// `poster_upload_id` links back to it. When `video_bound` is `false` the
/// video is left `confirmed` (never bound) to exercise the access query's
/// requirement that the linked video is itself bound. Returns the poster's
/// own `media_uploads.id`.
async fn insert_bound_poster(
    pool: &PgPool,
    user_id: Uuid,
    chatroom_id: Uuid,
    message_id: Uuid,
    video_bound: bool,
) -> TestResult<Uuid> {
    let poster_id = Uuid::new_v4();
    let video_id = Uuid::new_v4();
    let now = OffsetDateTime::now_utc();
    let confirmed_at = now - TimeDuration::minutes(2);
    let consumed_at = now - TimeDuration::minutes(1);
    let created_at = now - TimeDuration::minutes(3);
    let expires_at = now + TimeDuration::hours(1);
    let poster_object_key = format!("chat/{chatroom_id}/{poster_id}");
    sqlx::query(
        "INSERT INTO media_uploads \
             (id, user_id, object_key, scope, target_id, content_type, byte_size, \
              status, bound_message_id, confirmed_at, consumed_at, expires_at, created_at) \
         VALUES ($1, $2, $3, 'chat', $4, 'image/jpeg', 512, \
                 'bound', $5, $6, $7, $8, $9)",
    )
    .bind(poster_id)
    .bind(user_id)
    .bind(&poster_object_key)
    .bind(chatroom_id)
    .bind(message_id)
    .bind(confirmed_at)
    .bind(consumed_at)
    .bind(expires_at)
    .bind(created_at)
    .execute(pool)
    .await?;

    let video_object_key = format!("chat/{chatroom_id}/{video_id}");
    let (video_status, video_bound_message_id) = if video_bound {
        ("bound", Some(message_id))
    } else {
        ("confirmed", None)
    };
    sqlx::query(
        "INSERT INTO media_uploads \
             (id, user_id, object_key, scope, target_id, content_type, byte_size, \
              status, bound_message_id, consumed_at, confirmed_at, expires_at, created_at, \
              poster_upload_id) \
         VALUES ($1, $2, $3, 'chat', $4, 'video/mp4', 4096, \
                 $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(video_id)
    .bind(user_id)
    .bind(&video_object_key)
    .bind(chatroom_id)
    .bind(video_status)
    .bind(video_bound_message_id)
    .bind(video_bound.then_some(consumed_at))
    .bind(confirmed_at)
    .bind(expires_at)
    .bind(created_at)
    .bind(poster_id)
    .execute(pool)
    .await?;

    if video_bound {
        sqlx::query(
            "INSERT INTO message_media \
                 (id, message_id, media_upload_id, type, object_key, byte_size, position, \
                  created_at) \
             VALUES ($1, $2, $3, 'video/mp4', $4, 4096, 0, $5)",
        )
        .bind(Uuid::new_v4())
        .bind(message_id)
        .bind(video_id)
        .bind(&video_object_key)
        .bind(consumed_at)
        .execute(pool)
        .await?;
    }

    Ok(poster_id)
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
