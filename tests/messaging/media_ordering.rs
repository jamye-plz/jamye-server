//! Regression coverage for defects 2 and 3: sending a message with an
//! already-bound media attachment must produce a stored
//! `conversation_events`/`outbox_events` payload that carries the final
//! media list and the sender's display fields -- not the empty `media: []`
//! written before media binding used to run.  The HTTP send path also binds
//! up to `MAX_MEDIA_PER_MESSAGE` attachments in request order (it used to
//! reject any second one) and still rejects a fifth.

use std::sync::Arc;

use jamye_server::{
    adapters::postgres::{
        chatrooms::PostgresChatroomsRepository, media::PostgresMediaRepository,
        messaging::PostgresMessagingRepository, notifications::PostgresNotificationsRepository,
        topics::PostgresTopicsRepository, transactions::SqlxTransactionManager,
    },
    application::{
        chatrooms::ChatroomsService,
        messaging::{MessagingError, MessagingService, SendMessageInput, SendMessageOutcome},
        topics::{TopicsDependencies, TopicsService},
        transactions::{TransactionCompositionDependencies, TransactionCompositions},
    },
};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::postgres_support::{TestDatabase, TestResult};

#[tokio::test]
async fn http_send_with_confirmed_media_writes_final_media_and_sender_into_stored_event_and_outbox()
-> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;

    let (sender_id, chatroom_id) = insert_sender_with_main_chatroom(&pool).await?;
    let upload_id = insert_confirmed_image_upload(&pool, sender_id, chatroom_id).await?;
    let compositions = compositions(&pool);

    let outcome = compositions
        .send_message_http(
            sender_id,
            SendMessageInput {
                chatroom_id,
                client_msg_id: Uuid::new_v4(),
                body: None,
                media_upload_ids: vec![upload_id],
                idempotency_key: None,
            },
        )
        .await
        .map_err(|error| std::io::Error::other(format!("send_message_http failed: {error}")))?;

    let message = match outcome {
        SendMessageOutcome::Created(message) => message,
        SendMessageOutcome::Existing(_) => {
            return Err(std::io::Error::other("expected a newly Created message").into());
        }
    };
    assert_eq!(
        message.media.len(),
        1,
        "HTTP response must carry the bound attachment"
    );
    assert_eq!(message.media[0].media_upload_id, upload_id);
    assert_eq!(message.sender_nickname.as_deref(), Some("미디어 발신자"));

    let event_payload: Value =
        sqlx::query_scalar("SELECT payload FROM conversation_events WHERE payload ->> 'id' = $1")
            .bind(message.id.to_string())
            .fetch_one(&pool)
            .await?;
    let event_media = event_payload["media"]
        .as_array()
        .ok_or_else(|| std::io::Error::other("conversation_events payload.media is missing"))?;
    assert_eq!(
        event_media.len(),
        1,
        "conversation_events.payload.media must carry the bound attachment, not []"
    );
    assert_eq!(event_media[0]["media_upload_id"], upload_id.to_string());
    assert_eq!(
        event_payload["sender_nickname"],
        Value::String("미디어 발신자".to_owned())
    );
    assert_eq!(
        event_payload["sender_avatar_url"],
        Value::String("https://cdn.test/sender.png".to_owned())
    );

    let outbox_payload: Value = sqlx::query_scalar(
        "SELECT payload FROM outbox_events WHERE payload -> 'data' ->> 'id' = $1",
    )
    .bind(message.id.to_string())
    .fetch_one(&pool)
    .await?;
    let outbox_media = outbox_payload["data"]["media"]
        .as_array()
        .ok_or_else(|| std::io::Error::other("outbox_events payload.data.media is missing"))?;
    assert_eq!(
        outbox_media.len(),
        1,
        "outbox_events.payload.data.media must carry the bound attachment, not []"
    );
    assert_eq!(outbox_media[0]["media_upload_id"], upload_id.to_string());
    assert_eq!(
        outbox_payload["data"]["sender_nickname"],
        Value::String("미디어 발신자".to_owned())
    );

    database.dispose().await?;
    Ok(())
}

#[tokio::test]
async fn http_send_binds_up_to_four_visual_attachments_in_request_order() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let (sender_id, chatroom_id) = insert_sender_with_main_chatroom(&pool).await?;
    let mut upload_ids = Vec::new();
    for _ in 0..4 {
        upload_ids.push(insert_confirmed_image_upload(&pool, sender_id, chatroom_id).await?);
    }

    let outcome = compositions(&pool)
        .send_message_http(
            sender_id,
            SendMessageInput {
                chatroom_id,
                client_msg_id: Uuid::new_v4(),
                body: Some("사진 네 장".to_owned()),
                media_upload_ids: upload_ids.clone(),
                idempotency_key: None,
            },
        )
        .await
        .map_err(|error| std::io::Error::other(format!("send_message_http failed: {error}")))?;

    let message = match outcome {
        SendMessageOutcome::Created(message) => message,
        SendMessageOutcome::Existing(_) => {
            return Err(std::io::Error::other("expected a newly Created message").into());
        }
    };
    let bound = message
        .media
        .iter()
        .map(|attachment| attachment.media_upload_id)
        .collect::<Vec<_>>();
    assert_eq!(bound, upload_ids, "attachments keep the request order");

    let outbox_payload: Value = sqlx::query_scalar(
        "SELECT payload FROM outbox_events WHERE payload -> 'data' ->> 'id' = $1",
    )
    .bind(message.id.to_string())
    .fetch_one(&pool)
    .await?;
    let outbox_media = outbox_payload["data"]["media"]
        .as_array()
        .ok_or_else(|| std::io::Error::other("outbox_events payload.data.media is missing"))?;
    assert_eq!(
        outbox_media.len(),
        4,
        "the outbox carries all four attachments"
    );

    database.dispose().await?;
    Ok(())
}

#[tokio::test]
async fn http_send_retry_with_four_attachments_returns_the_existing_message_unchanged() -> TestResult
{
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let (sender_id, chatroom_id) = insert_sender_with_main_chatroom(&pool).await?;
    let mut upload_ids = Vec::new();
    for _ in 0..4 {
        upload_ids.push(insert_confirmed_image_upload(&pool, sender_id, chatroom_id).await?);
    }
    let input = SendMessageInput {
        chatroom_id,
        client_msg_id: Uuid::new_v4(),
        body: None,
        media_upload_ids: upload_ids.clone(),
        idempotency_key: None,
    };
    let compositions = compositions(&pool);

    let created = match compositions
        .send_message_http(sender_id, input.clone())
        .await
        .map_err(|error| std::io::Error::other(format!("first send failed: {error}")))?
    {
        SendMessageOutcome::Created(message) => message,
        SendMessageOutcome::Existing(_) => {
            return Err(std::io::Error::other("expected the first send to be Created").into());
        }
    };
    // The app's outbox resends the same client_msg_id when a response is lost.
    let replayed = match compositions
        .send_message_http(sender_id, input)
        .await
        .map_err(|error| std::io::Error::other(format!("retry failed: {error}")))?
    {
        SendMessageOutcome::Existing(message) => message,
        SendMessageOutcome::Created(_) => {
            return Err(std::io::Error::other("expected the retry to return Existing").into());
        }
    };
    assert_eq!(
        replayed, created,
        "a retry returns the stored message as-is"
    );
    let bound = replayed
        .media
        .iter()
        .map(|attachment| attachment.media_upload_id)
        .collect::<Vec<_>>();
    assert_eq!(bound, upload_ids, "a retry keeps the request order");

    let messages: i64 = sqlx::query_scalar("SELECT count(*) FROM messages WHERE chatroom_id = $1")
        .bind(chatroom_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(messages, 1, "a retry never duplicates the message");
    let attachments: i64 =
        sqlx::query_scalar("SELECT count(*) FROM message_media WHERE message_id = $1")
            .bind(created.id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(attachments, 4, "a retry never duplicates its attachments");

    database.dispose().await?;
    Ok(())
}

#[tokio::test]
async fn http_send_rejects_a_fifth_attachment_without_persisting_a_message() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let (sender_id, chatroom_id) = insert_sender_with_main_chatroom(&pool).await?;
    let mut upload_ids = Vec::new();
    for _ in 0..5 {
        upload_ids.push(insert_confirmed_image_upload(&pool, sender_id, chatroom_id).await?);
    }

    let result = compositions(&pool)
        .send_message_http(
            sender_id,
            SendMessageInput {
                chatroom_id,
                client_msg_id: Uuid::new_v4(),
                body: None,
                media_upload_ids: upload_ids,
                idempotency_key: None,
            },
        )
        .await;
    assert_eq!(result, Err(MessagingError::MediaNotAvailable));
    let messages: i64 = sqlx::query_scalar("SELECT count(*) FROM messages WHERE chatroom_id = $1")
        .bind(chatroom_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(messages, 0, "a rejected send persists nothing");

    database.dispose().await?;
    Ok(())
}

async fn insert_sender_with_main_chatroom(pool: &PgPool) -> TestResult<(Uuid, Uuid)> {
    let sender_id = insert_user(pool, "미디어 발신자", Some("https://cdn.test/sender.png")).await?;
    let group_id = Uuid::new_v4();
    sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, '미디어 그룹', $2)")
        .bind(group_id)
        .bind(sender_id)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, 'owner')",
    )
    .bind(Uuid::new_v4())
    .bind(group_id)
    .bind(sender_id)
    .execute(pool)
    .await?;
    let chatroom_id = Uuid::new_v4();
    sqlx::query("INSERT INTO chatrooms (id, group_id, type) VALUES ($1, $2, 'main')")
        .bind(chatroom_id)
        .bind(group_id)
        .execute(pool)
        .await?;
    Ok((sender_id, chatroom_id))
}

async fn insert_confirmed_image_upload(
    pool: &PgPool,
    sender_id: Uuid,
    chatroom_id: Uuid,
) -> TestResult<Uuid> {
    let upload_id = Uuid::new_v4();
    sqlx::query(
        "WITH stamped AS (SELECT clock_timestamp() AS now) \
         INSERT INTO media_uploads \
             (id, user_id, object_key, scope, target_id, content_type, byte_size, \
              status, confirmed_at, expires_at, created_at) \
         SELECT $1, $2, $3, 'chat', $4, 'image/jpeg', 2048, \
                'confirmed', stamped.now, stamped.now + INTERVAL '1 hour', stamped.now \
         FROM stamped",
    )
    .bind(upload_id)
    .bind(sender_id)
    .bind(format!("chat/{chatroom_id}/{upload_id}"))
    .bind(chatroom_id)
    .execute(pool)
    .await?;
    Ok(upload_id)
}

fn compositions(pool: &PgPool) -> TransactionCompositions {
    let transactions = Arc::new(SqlxTransactionManager::new(pool.clone()));
    TransactionCompositions::new(TransactionCompositionDependencies {
        transactions: transactions.clone(),
        messaging: Arc::new(MessagingService::new(
            transactions.clone(),
            Arc::new(PostgresMessagingRepository::new(pool.clone())),
        )),
        media: Arc::new(PostgresMediaRepository::new(pool.clone())),
        topics: Arc::new(TopicsService::new(TopicsDependencies {
            transactions: transactions.clone(),
            repository: Arc::new(PostgresTopicsRepository::new(pool.clone())),
        })),
        chatrooms: Arc::new(ChatroomsService::new(
            transactions,
            Arc::new(PostgresChatroomsRepository::new(pool.clone())),
        )),
        notifications: Arc::new(PostgresNotificationsRepository::new(pool.clone())),
    })
}

async fn insert_user(pool: &PgPool, nickname: &str, avatar_url: Option<&str>) -> TestResult<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, nickname, avatar_url) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(nickname)
        .bind(avatar_url)
        .execute(pool)
        .await?;
    Ok(id)
}
