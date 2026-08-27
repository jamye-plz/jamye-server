use std::{collections::BTreeMap, io, time::Duration};

use jamye_server::{
    adapters::{
        object_storage::media::S3MediaObjectStorage,
        postgres::{media::PostgresMediaRepository, transactions::SqlxTransactionManager},
    },
    config::{
        AppEnvironment,
        object_storage::{ObjectStorageConfig, ObjectStorageConfigInput},
    },
    domain::media::{MediaKind, MediaScope, PRESIGNED_PUT_TTL_SECONDS},
    ports::{
        media::{CreateUploadIntentCommand, MediaRepository, MediaRepositoryError},
        object_storage::{MediaObjectStorage, PresignPutRequest},
        transactions::TransactionManager,
    },
};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

const PRIVATE_BUCKET: &str = "jamye-private-media";
const ACCESS_KEY: &str = "task-8-presign-access-key";
const SECRET_KEY: &str = "task-8-presign-secret-key";

#[tokio::test]
async fn postgres_intent_insert_authorizes_targets_and_obeys_caller_transaction() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let actor_id = insert_user(&pool, "업로드 멤버").await?;
    let outsider_id = insert_user(&pool, "업로드 외부인").await?;
    let group_id = Uuid::new_v4();
    sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, '미디어 그룹', $2)")
        .bind(group_id)
        .bind(actor_id)
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, 'owner')",
    )
    .bind(Uuid::new_v4())
    .bind(group_id)
    .bind(actor_id)
    .execute(&pool)
    .await?;
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
    .bind(actor_id)
    .bind(Uuid::new_v4())
    .bind("a".repeat(64))
    .execute(&pool)
    .await?;

    let transactions = SqlxTransactionManager::new(pool.clone());
    let repository = PostgresMediaRepository::new(pool.clone());

    let chat = command(
        actor_id,
        MediaScope::Chat,
        chatroom_id,
        "image/jpeg",
        1_024,
        Some(String::new()),
    );
    let mut transaction = transactions.begin().await?;
    let stored_chat = match repository
        .create_upload_intent(transaction.as_mut(), &chat)
        .await
    {
        Ok(stored) => stored,
        Err(error) => {
            transactions.rollback(transaction).await?;
            pool.close().await;
            database.dispose().await?;
            return Err(error.into());
        }
    };
    transactions.commit(transaction).await?;
    assert_record(&stored_chat, &chat);
    assert_eq!(stored_chat.filename.as_deref(), Some(""));

    let topic = command(
        actor_id,
        MediaScope::Topic,
        topic_id,
        "image/png",
        2_048,
        Some("가".repeat(255)),
    );
    let mut transaction = transactions.begin().await?;
    let stored_topic = repository
        .create_upload_intent(transaction.as_mut(), &topic)
        .await?;
    transactions.commit(transaction).await?;
    assert_record(&stored_topic, &topic);

    let denied = command(
        outsider_id,
        MediaScope::Chat,
        chatroom_id,
        "image/webp",
        512,
        None,
    );
    let mut transaction = transactions.begin().await?;
    assert_eq!(
        repository
            .create_upload_intent(transaction.as_mut(), &denied)
            .await,
        Err(MediaRepositoryError::TargetNotAccessible)
    );
    transactions.rollback(transaction).await?;

    let missing = command(
        actor_id,
        MediaScope::Topic,
        Uuid::new_v4(),
        "image/gif",
        256,
        None,
    );
    let mut transaction = transactions.begin().await?;
    assert_eq!(
        repository
            .create_upload_intent(transaction.as_mut(), &missing)
            .await,
        Err(MediaRepositoryError::TargetNotAccessible)
    );
    transactions.rollback(transaction).await?;

    let rolled_back = command(
        actor_id,
        MediaScope::Chat,
        chatroom_id,
        "video/mp4",
        4_096,
        Some("취소.mp4".to_owned()),
    );
    let mut transaction = transactions.begin().await?;
    repository
        .create_upload_intent(transaction.as_mut(), &rolled_back)
        .await?;
    transactions.rollback(transaction).await?;

    let rows: Vec<(Uuid, String, Option<String>, i64, String)> = sqlx::query_as(
        "SELECT id, object_key, filename, byte_size, status \
         FROM media_uploads ORDER BY created_at, id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|row| row.0 == chat.id));
    assert!(rows.iter().any(|row| row.0 == topic.id));
    assert!(!rows.iter().any(|row| row.0 == denied.id));
    assert!(!rows.iter().any(|row| row.0 == missing.id));
    assert!(!rows.iter().any(|row| row.0 == rolled_back.id));
    assert!(rows.iter().all(|row| row.4 == "pending"));

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn sdk_put_presign_uses_public_path_style_origin_and_binds_upload_constraints() -> TestResult
{
    let config = object_storage_config()?;
    let storage = S3MediaObjectStorage::new(&config);
    let object_key = format!("chat/{}/{}", Uuid::new_v4(), Uuid::new_v4());
    let request = PresignPutRequest {
        object_key: object_key.clone(),
        content_type: "audio/mp4".to_owned(),
        byte_size: 15_728_640,
        expires_in: Duration::from_secs(PRESIGNED_PUT_TTL_SECONDS),
    };

    let presigned = storage.presign_put(&request).await?;
    assert_eq!(presigned.expires_in, request.expires_in);
    assert!(!presigned.url.contains(SECRET_KEY));

    let url = Url::parse(&presigned.url)?;
    assert_eq!(url.scheme(), "http");
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    assert_eq!(url.port(), Some(45679));
    assert_eq!(url.path(), format!("/{PRIVATE_BUCKET}/{object_key}"));
    let query = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        query.get("X-Amz-Algorithm").map(String::as_str),
        Some("AWS4-HMAC-SHA256")
    );
    assert_eq!(query.get("X-Amz-Expires").map(String::as_str), Some("3600"));
    assert!(
        query
            .get("X-Amz-Credential")
            .is_some_and(|value| value.starts_with(ACCESS_KEY))
    );
    let signed_headers = query
        .get("X-Amz-SignedHeaders")
        .ok_or_else(|| io::Error::other("presign omitted X-Amz-SignedHeaders"))?;
    for required in ["content-length", "content-type", "host"] {
        assert!(
            signed_headers.split(';').any(|value| value == required),
            "presign did not bind {required}: {signed_headers}"
        );
    }
    let signature = query
        .get("X-Amz-Signature")
        .ok_or_else(|| io::Error::other("presign omitted X-Amz-Signature"))?;
    assert_eq!(signature.len(), 64);
    assert!(signature.bytes().all(|byte| byte.is_ascii_hexdigit()));
    Ok(())
}

fn command(
    user_id: Uuid,
    scope: MediaScope,
    target_id: Uuid,
    content_type: &str,
    byte_size: u64,
    filename: Option<String>,
) -> CreateUploadIntentCommand {
    let id = Uuid::new_v4();
    let prefix = match scope {
        MediaScope::Chat => "chat",
        MediaScope::Topic => "topics",
    };
    CreateUploadIntentCommand {
        id,
        user_id,
        scope,
        target_id,
        object_key: format!("{prefix}/{target_id}/{id}"),
        kind: match content_type {
            "video/mp4" => MediaKind::Video,
            value if value.starts_with("audio/") => MediaKind::Audio,
            _ => MediaKind::Image,
        },
        content_type: content_type.to_owned(),
        byte_size,
        filename,
        expires_in: Duration::from_secs(PRESIGNED_PUT_TTL_SECONDS),
    }
}

fn assert_record(
    record: &jamye_server::ports::media::UploadIntentRecord,
    command: &CreateUploadIntentCommand,
) {
    assert_eq!(record.id, command.id);
    assert_eq!(record.user_id, command.user_id);
    assert_eq!(record.scope, command.scope);
    assert_eq!(record.target_id, command.target_id);
    assert_eq!(record.object_key, command.object_key);
    assert_eq!(record.kind, command.kind);
    assert_eq!(record.content_type, command.content_type);
    assert_eq!(record.byte_size, command.byte_size);
    assert_eq!(record.filename, command.filename);
    assert_eq!(
        (record.expires_at - record.created_at).whole_seconds(),
        i64::try_from(command.expires_in.as_secs()).unwrap_or(i64::MAX)
    );
    assert!(record.created_at > OffsetDateTime::UNIX_EPOCH);
}

async fn insert_user(pool: &sqlx::PgPool, nickname: &str) -> TestResult<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, $2)")
        .bind(id)
        .bind(nickname)
        .execute(pool)
        .await?;
    Ok(id)
}

fn object_storage_config() -> TestResult<ObjectStorageConfig> {
    ObjectStorageConfig::resolve(
        AppEnvironment::Test,
        ObjectStorageConfigInput {
            endpoint: Some("http://127.0.0.1:45678".to_owned()),
            public_endpoint: Some("http://127.0.0.1:45679".to_owned()),
            region: Some("us-east-1".to_owned()),
            bucket: Some(PRIVATE_BUCKET.to_owned()),
            access_key_id: Some(ACCESS_KEY.to_owned()),
            secret_access_key: Some(SECRET_KEY.to_owned()),
        },
    )?
    .ok_or_else(|| io::Error::other("complete object-storage config resolved absent").into())
}
