use std::{collections::HashSet, io};

use jamye_server::{
    adapters::postgres::{media::PostgresMediaRepository, transactions::SqlxTransactionManager},
    domain::media::{FinalizedObject, MediaKind},
    ports::{
        media::{
            BindMessageMediaCommand, BindMessageMediaItem, MediaRepository, MediaRepositoryError,
        },
        transactions::TransactionManager,
    },
};
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

const IMAGE_BYTES: u64 = 1_024;
const AUDIO_BYTES: u64 = 8_192;
const AUDIO_DURATION: u64 = 38;
const FILENAME: &str = " 여름/기록 \"최종\".jpg ";

type StoredAttachmentRow = (Uuid, String, String, i64, Option<i32>, Option<String>, i32);

#[tokio::test]
async fn postgres_message_binding_copies_db_owned_metadata_in_request_order() -> TestResult {
    let fixture = MessageBindingFixture::new().await?;
    let result = ordered_binding_case(&fixture).await;
    fixture.dispose().await?;
    result
}

#[tokio::test]
async fn postgres_message_binding_accepts_bodyless_exactly_one_audio() -> TestResult {
    let fixture = MessageBindingFixture::new().await?;
    let result = voice_binding_case(&fixture).await;
    fixture.dispose().await?;
    result
}

#[tokio::test]
async fn postgres_message_binding_rechecks_actor_target_and_capability_state() -> TestResult {
    let fixture = MessageBindingFixture::new().await?;
    let result = authorization_and_state_case(&fixture).await;
    fixture.dispose().await?;
    result
}

#[tokio::test]
async fn postgres_message_binding_obeys_rollback_and_exact_retry() -> TestResult {
    let fixture = MessageBindingFixture::new().await?;
    let result = rollback_and_retry_case(&fixture).await;
    fixture.dispose().await?;
    result
}

#[tokio::test]
async fn postgres_message_binding_rejects_provider_metadata_drift() -> TestResult {
    let fixture = MessageBindingFixture::new().await?;
    let result = metadata_drift_case(&fixture).await;
    fixture.dispose().await?;
    result
}

async fn ordered_binding_case(fixture: &MessageBindingFixture) -> TestResult {
    let uploads = [
        fixture
            .insert_upload(UploadSpec::confirmed(
                fixture.actor_id,
                fixture.chatroom_id,
                "image/jpeg",
                IMAGE_BYTES,
                None,
                Some(FILENAME),
            ))
            .await?,
        fixture
            .insert_upload(UploadSpec::confirmed(
                fixture.actor_id,
                fixture.chatroom_id,
                "video/mp4",
                4_096,
                None,
                Some("clip.mp4"),
            ))
            .await?,
        fixture
            .insert_upload(UploadSpec::confirmed(
                fixture.actor_id,
                fixture.chatroom_id,
                "image/png",
                2_048,
                None,
                None,
            ))
            .await?,
        fixture
            .insert_upload(UploadSpec::confirmed(
                fixture.actor_id,
                fixture.chatroom_id,
                "image/webp",
                3_072,
                None,
                Some("four.webp"),
            ))
            .await?,
    ];
    let ordered = vec![
        uploads[2].clone(),
        uploads[0].clone(),
        uploads[3].clone(),
        uploads[1].clone(),
    ];
    let command = binding_command(fixture, fixture.message_id, &ordered);
    let repository = fixture.repository();
    let transactions = fixture.transactions();
    let mut transaction = transactions.begin().await?;
    let attachments = repository
        .bind_message_media(transaction.as_mut(), &command)
        .await?;
    transactions.commit(transaction).await?;

    assert_eq!(attachments.len(), ordered.len());
    assert_eq!(
        attachments
            .iter()
            .map(|attachment| attachment.id)
            .collect::<HashSet<_>>()
            .len(),
        ordered.len()
    );
    for (position, (attachment, upload)) in attachments.iter().zip(&ordered).enumerate() {
        assert_eq!(attachment.media_upload_id, upload.id);
        assert_eq!(attachment.content_type, upload.finalized.content_type);
        assert_eq!(attachment.byte_size, upload.finalized.byte_size);
        assert_eq!(attachment.duration, upload.finalized.duration_seconds);
        assert_eq!(attachment.filename, upload.filename);
        assert_eq!(attachment.width, None);
        assert_eq!(attachment.height, None);
        assert_eq!(attachment.position, u8::try_from(position)?);
        assert_eq!(
            upload_state(&fixture.pool, upload.id).await?,
            ("bound".to_owned(), Some(fixture.message_id), true)
        );
    }

    let stored: Vec<StoredAttachmentRow> = sqlx::query_as(
        "SELECT media_upload_id, object_key, type, byte_size, duration, filename, position \
         FROM message_media WHERE message_id = $1 ORDER BY position",
    )
    .bind(fixture.message_id)
    .fetch_all(&fixture.pool)
    .await?;
    assert_eq!(stored.len(), ordered.len());
    for (position, (row, upload)) in stored.iter().zip(&ordered).enumerate() {
        assert_eq!(row.0, upload.id);
        assert_eq!(row.1, upload.object_key);
        assert_eq!(row.2, upload.finalized.content_type);
        assert_eq!(row.3, i64::try_from(upload.finalized.byte_size)?);
        assert_eq!(
            row.4,
            upload
                .finalized
                .duration_seconds
                .map(i32::try_from)
                .transpose()?
        );
        assert_eq!(row.5, upload.filename);
        assert_eq!(row.6, i32::try_from(position)?);
    }
    Ok(())
}

async fn voice_binding_case(fixture: &MessageBindingFixture) -> TestResult {
    let message_id = fixture.insert_message(None).await?;
    let upload = fixture
        .insert_upload(UploadSpec::confirmed(
            fixture.actor_id,
            fixture.chatroom_id,
            "audio/ogg",
            AUDIO_BYTES,
            Some(AUDIO_DURATION),
            Some("voice.ogg"),
        ))
        .await?;
    let command = binding_command(fixture, message_id, std::slice::from_ref(&upload));
    let repository = fixture.repository();
    let transactions = fixture.transactions();
    let mut transaction = transactions.begin().await?;
    let attachments = repository
        .bind_message_media(transaction.as_mut(), &command)
        .await?;
    transactions.commit(transaction).await?;

    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].media_upload_id, upload.id);
    assert_eq!(attachments[0].content_type, "audio/ogg");
    assert_eq!(attachments[0].byte_size, AUDIO_BYTES);
    assert_eq!(attachments[0].duration, Some(AUDIO_DURATION));
    assert_eq!(attachments[0].position, 0);
    Ok(())
}

async fn authorization_and_state_case(fixture: &MessageBindingFixture) -> TestResult {
    let fresh = fixture
        .insert_upload(UploadSpec::confirmed(
            fixture.actor_id,
            fixture.chatroom_id,
            "image/jpeg",
            IMAGE_BYTES,
            None,
            Some(FILENAME),
        ))
        .await?;
    let valid = binding_command(fixture, fixture.message_id, std::slice::from_ref(&fresh));

    // Establish that this fixture is otherwise bindable, then restore it for the denial cases.
    let repository = fixture.repository();
    let transactions = fixture.transactions();
    let mut transaction = transactions.begin().await?;
    repository
        .bind_message_media(transaction.as_mut(), &valid)
        .await?;
    transactions.rollback(transaction).await?;

    let mut wrong_actor = valid.clone();
    wrong_actor.actor_id = fixture.other_member_id;
    assert_binding_error(
        fixture,
        &wrong_actor,
        MediaRepositoryError::TargetNotAccessible,
    )
    .await?;

    let mut wrong_target = valid.clone();
    wrong_target.chatroom_id = Uuid::new_v4();
    assert_binding_error(
        fixture,
        &wrong_target,
        MediaRepositoryError::TargetNotAccessible,
    )
    .await?;

    let mut missing_message = valid.clone();
    missing_message.message_id = Uuid::new_v4();
    assert_binding_error(
        fixture,
        &missing_message,
        MediaRepositoryError::TargetNotAccessible,
    )
    .await?;

    let foreign = fixture
        .insert_upload(UploadSpec::confirmed(
            fixture.other_member_id,
            fixture.chatroom_id,
            "image/png",
            IMAGE_BYTES,
            None,
            None,
        ))
        .await?;
    assert_binding_error(
        fixture,
        &binding_command(fixture, fixture.message_id, &[foreign]),
        MediaRepositoryError::TargetNotAccessible,
    )
    .await?;

    let expired = fixture
        .insert_upload(UploadSpec::expired(
            fixture.actor_id,
            fixture.chatroom_id,
            "image/webp",
            IMAGE_BYTES,
        ))
        .await?;
    assert_binding_error(
        fixture,
        &binding_command(fixture, fixture.message_id, &[expired]),
        MediaRepositoryError::FinalizeConflict,
    )
    .await?;

    let pending = fixture
        .insert_upload(UploadSpec::pending(
            fixture.actor_id,
            fixture.chatroom_id,
            "image/gif",
            IMAGE_BYTES,
        ))
        .await?;
    assert_binding_error(
        fixture,
        &binding_command(fixture, fixture.message_id, &[pending]),
        MediaRepositoryError::FinalizeConflict,
    )
    .await?;

    assert_eq!(
        message_media_count(&fixture.pool, fixture.message_id).await?,
        0
    );
    assert_eq!(
        upload_state(&fixture.pool, fresh.id).await?,
        ("confirmed".to_owned(), None, false)
    );
    Ok(())
}

async fn rollback_and_retry_case(fixture: &MessageBindingFixture) -> TestResult {
    let first = fixture
        .insert_upload(UploadSpec::confirmed(
            fixture.actor_id,
            fixture.chatroom_id,
            "image/jpeg",
            IMAGE_BYTES,
            None,
            Some(FILENAME),
        ))
        .await?;
    let second = fixture
        .insert_upload(UploadSpec::confirmed(
            fixture.actor_id,
            fixture.chatroom_id,
            "video/mp4",
            4_096,
            None,
            Some("clip.mp4"),
        ))
        .await?;
    let ordered = vec![first.clone(), second.clone()];
    let command = binding_command(fixture, fixture.message_id, &ordered);
    let repository = fixture.repository();
    let transactions = fixture.transactions();

    let mut rolled_back_transaction = transactions.begin().await?;
    repository
        .bind_message_media(rolled_back_transaction.as_mut(), &command)
        .await?;
    transactions.rollback(rolled_back_transaction).await?;
    assert_eq!(
        message_media_count(&fixture.pool, fixture.message_id).await?,
        0
    );
    for upload in &ordered {
        assert_eq!(
            upload_state(&fixture.pool, upload.id).await?,
            ("confirmed".to_owned(), None, false)
        );
    }

    let mut committed_transaction = transactions.begin().await?;
    let canonical = repository
        .bind_message_media(committed_transaction.as_mut(), &command)
        .await?;
    transactions.commit(committed_transaction).await?;
    assert_eq!(
        message_media_count(&fixture.pool, fixture.message_id).await?,
        2
    );

    let mut retry_transaction = transactions.begin().await?;
    let retry = repository
        .bind_message_media(retry_transaction.as_mut(), &command)
        .await?;
    transactions.commit(retry_transaction).await?;
    assert_eq!(retry, canonical);
    assert_eq!(
        message_media_count(&fixture.pool, fixture.message_id).await?,
        2
    );

    let reversed = binding_command(
        fixture,
        fixture.message_id,
        &[second.clone(), first.clone()],
    );
    assert_binding_error(fixture, &reversed, MediaRepositoryError::FinalizeConflict).await?;

    let other_message_id = fixture.insert_message(Some("다른 메시지")).await?;
    let reused = binding_command(fixture, other_message_id, &ordered);
    assert_binding_error(fixture, &reused, MediaRepositoryError::FinalizeConflict).await?;
    assert_eq!(
        message_media_count(&fixture.pool, fixture.message_id).await?,
        2
    );
    assert_eq!(
        message_media_count(&fixture.pool, other_message_id).await?,
        0
    );
    Ok(())
}

async fn metadata_drift_case(fixture: &MessageBindingFixture) -> TestResult {
    let message_id = fixture.insert_message(None).await?;
    let upload = fixture
        .insert_upload(UploadSpec::confirmed(
            fixture.actor_id,
            fixture.chatroom_id,
            "audio/ogg",
            AUDIO_BYTES,
            Some(AUDIO_DURATION),
            Some("voice.ogg"),
        ))
        .await?;
    let valid = binding_command(fixture, message_id, std::slice::from_ref(&upload));

    // Establish that the exact provider observation is accepted, then roll it back.
    let repository = fixture.repository();
    let transactions = fixture.transactions();
    let mut transaction = transactions.begin().await?;
    repository
        .bind_message_media(transaction.as_mut(), &valid)
        .await?;
    transactions.rollback(transaction).await?;

    let mut wrong_kind = valid.clone();
    wrong_kind.media[0].finalized.kind = MediaKind::Image;
    assert_binding_error(fixture, &wrong_kind, MediaRepositoryError::FinalizeConflict).await?;

    let mut wrong_type = valid.clone();
    wrong_type.media[0].finalized.content_type = "audio/mp4".to_owned();
    assert_binding_error(fixture, &wrong_type, MediaRepositoryError::FinalizeConflict).await?;

    let mut wrong_size = valid.clone();
    wrong_size.media[0].finalized.byte_size += 1;
    assert_binding_error(fixture, &wrong_size, MediaRepositoryError::FinalizeConflict).await?;

    let mut wrong_duration = valid;
    wrong_duration.media[0].finalized.duration_seconds = Some(AUDIO_DURATION - 1);
    assert_binding_error(
        fixture,
        &wrong_duration,
        MediaRepositoryError::FinalizeConflict,
    )
    .await?;

    assert_eq!(message_media_count(&fixture.pool, message_id).await?, 0);
    assert_eq!(
        upload_state(&fixture.pool, upload.id).await?,
        ("confirmed".to_owned(), None, false)
    );
    Ok(())
}

async fn assert_binding_error(
    fixture: &MessageBindingFixture,
    command: &BindMessageMediaCommand,
    expected: MediaRepositoryError,
) -> TestResult {
    let repository = fixture.repository();
    let transactions = fixture.transactions();
    let mut transaction = transactions.begin().await?;
    let result = repository
        .bind_message_media(transaction.as_mut(), command)
        .await;
    transactions.rollback(transaction).await?;
    if result == Err(expected) {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "unexpected message binding result: {result:?}; expected {expected:?}"
        ))
        .into())
    }
}

fn binding_command(
    fixture: &MessageBindingFixture,
    message_id: Uuid,
    uploads: &[SeededUpload],
) -> BindMessageMediaCommand {
    BindMessageMediaCommand {
        actor_id: fixture.actor_id,
        chatroom_id: fixture.chatroom_id,
        message_id,
        media: uploads
            .iter()
            .map(|upload| BindMessageMediaItem {
                upload_id: upload.id,
                finalized: upload.finalized.clone(),
            })
            .collect(),
    }
}

#[derive(Clone, Debug)]
struct SeededUpload {
    id: Uuid,
    object_key: String,
    finalized: FinalizedObject,
    filename: Option<String>,
}

#[derive(Clone, Copy, Debug)]
enum SeedStatus {
    Confirmed,
    Expired,
    Pending,
}

#[derive(Clone, Copy, Debug)]
struct UploadSpec<'a> {
    user_id: Uuid,
    target_id: Uuid,
    content_type: &'a str,
    byte_size: u64,
    duration_seconds: Option<u64>,
    filename: Option<&'a str>,
    status: SeedStatus,
}

impl<'a> UploadSpec<'a> {
    fn confirmed(
        user_id: Uuid,
        target_id: Uuid,
        content_type: &'a str,
        byte_size: u64,
        duration_seconds: Option<u64>,
        filename: Option<&'a str>,
    ) -> Self {
        Self {
            user_id,
            target_id,
            content_type,
            byte_size,
            duration_seconds,
            filename,
            status: SeedStatus::Confirmed,
        }
    }

    fn expired(user_id: Uuid, target_id: Uuid, content_type: &'a str, byte_size: u64) -> Self {
        Self {
            user_id,
            target_id,
            content_type,
            byte_size,
            duration_seconds: None,
            filename: None,
            status: SeedStatus::Expired,
        }
    }

    fn pending(user_id: Uuid, target_id: Uuid, content_type: &'a str, byte_size: u64) -> Self {
        Self {
            user_id,
            target_id,
            content_type,
            byte_size,
            duration_seconds: None,
            filename: None,
            status: SeedStatus::Pending,
        }
    }
}

struct MessageBindingFixture {
    database: TestDatabase,
    pool: PgPool,
    actor_id: Uuid,
    other_member_id: Uuid,
    chatroom_id: Uuid,
    message_id: Uuid,
}

impl MessageBindingFixture {
    async fn new() -> TestResult<Self> {
        let database = TestDatabase::migrated().await?;
        let pool = database.pool()?;
        let actor_id = insert_user(&pool, "메시지 작성자").await?;
        let other_member_id = insert_user(&pool, "다른 그룹 멤버").await?;
        let group_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO groups (id, name, owner_id) VALUES ($1, '메시지 미디어 그룹', $2)",
        )
        .bind(group_id)
        .bind(actor_id)
        .execute(&pool)
        .await?;
        for (user_id, role) in [(actor_id, "owner"), (other_member_id, "member")] {
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
        let message_id = insert_message(&pool, chatroom_id, actor_id, Some("첨부 메시지")).await?;

        Ok(Self {
            database,
            pool,
            actor_id,
            other_member_id,
            chatroom_id,
            message_id,
        })
    }

    fn repository(&self) -> PostgresMediaRepository {
        PostgresMediaRepository::new(self.pool.clone())
    }

    fn transactions(&self) -> SqlxTransactionManager {
        SqlxTransactionManager::new(self.pool.clone())
    }

    async fn insert_message(&self, body: Option<&str>) -> TestResult<Uuid> {
        insert_message(&self.pool, self.chatroom_id, self.actor_id, body).await
    }

    async fn insert_upload(&self, spec: UploadSpec<'_>) -> TestResult<SeededUpload> {
        let id = Uuid::new_v4();
        let object_key = format!("chat/{}/{id}", spec.target_id);
        let now = OffsetDateTime::now_utc();
        let (status, created_at, confirmed_at, expires_at) = match spec.status {
            SeedStatus::Confirmed => (
                "confirmed",
                now - Duration::minutes(1),
                Some(now),
                now + Duration::hours(1),
            ),
            SeedStatus::Expired => (
                "confirmed",
                now - Duration::hours(2),
                Some(now - Duration::minutes(90)),
                now - Duration::hours(1),
            ),
            SeedStatus::Pending => (
                "pending",
                now - Duration::minutes(1),
                None,
                now + Duration::hours(1),
            ),
        };
        sqlx::query(
            "INSERT INTO media_uploads \
                 (id, user_id, object_key, scope, target_id, content_type, byte_size, duration, \
                  filename, status, confirmed_at, expires_at, created_at) \
             VALUES ($1, $2, $3, 'chat', $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(id)
        .bind(spec.user_id)
        .bind(&object_key)
        .bind(spec.target_id)
        .bind(spec.content_type)
        .bind(i64::try_from(spec.byte_size)?)
        .bind(spec.duration_seconds.map(i32::try_from).transpose()?)
        .bind(spec.filename)
        .bind(status)
        .bind(confirmed_at)
        .bind(expires_at)
        .bind(created_at)
        .execute(&self.pool)
        .await?;

        Ok(SeededUpload {
            id,
            object_key,
            finalized: FinalizedObject {
                kind: media_kind(spec.content_type),
                content_type: spec.content_type.to_owned(),
                byte_size: spec.byte_size,
                duration_seconds: spec.duration_seconds,
            },
            filename: spec.filename.map(ToOwned::to_owned),
        })
    }

    async fn dispose(self) -> TestResult {
        let Self { database, pool, .. } = self;
        pool.close().await;
        database.dispose().await
    }
}

fn media_kind(content_type: &str) -> MediaKind {
    match content_type {
        "video/mp4" => MediaKind::Video,
        value if value.starts_with("audio/") => MediaKind::Audio,
        _ => MediaKind::Image,
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

async fn insert_message(
    pool: &PgPool,
    chatroom_id: Uuid,
    actor_id: Uuid,
    body: Option<&str>,
) -> TestResult<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO messages (id, chatroom_id, sender_id, client_msg_id, body, type) \
         VALUES ($1, $2, $3, $4, $5, 'user')",
    )
    .bind(id)
    .bind(chatroom_id)
    .bind(actor_id)
    .bind(Uuid::new_v4())
    .bind(body)
    .execute(pool)
    .await?;
    Ok(id)
}

async fn upload_state(pool: &PgPool, upload_id: Uuid) -> TestResult<(String, Option<Uuid>, bool)> {
    Ok(sqlx::query_as(
        "SELECT status, bound_message_id, consumed_at IS NOT NULL \
         FROM media_uploads WHERE id = $1",
    )
    .bind(upload_id)
    .fetch_one(pool)
    .await?)
}

async fn message_media_count(pool: &PgPool, message_id: Uuid) -> TestResult<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM message_media WHERE message_id = $1")
            .bind(message_id)
            .fetch_one(pool)
            .await?,
    )
}
