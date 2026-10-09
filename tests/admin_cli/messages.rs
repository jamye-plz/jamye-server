//! `messages delete`: the moderator deletion shares the author-delete semantics (soft delete,
//! tombstone events for sync, media cleanup queue), is idempotent, and actions linked reports.

use std::sync::Arc;

use jamye_server::{
    adapters::postgres::{
        messaging::PostgresMessagingRepository, transactions::SqlxTransactionManager,
    },
    application::{
        auth::AccessIdentity,
        messaging::{
            DeleteMessageInput, DeltaInput, MessagingService, SendMessageInput, SendMessageOutcome,
        },
    },
    domain::messaging::DeltaItem,
};
use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    TestResult,
    harness::{AdminHarness, ReportFixture, assert_failure, assert_usage, report_status},
    support::{GroupFixture, count, insert_group, insert_user},
};

const BODY: &str = "the offending words";

struct Fixture {
    author: Uuid,
    viewer: Uuid,
    group: GroupFixture,
    messaging: MessagingService,
}

async fn fixture(harness: &AdminHarness) -> TestResult<Fixture> {
    let author = insert_user(&harness.pool, "author").await?;
    let viewer = insert_user(&harness.pool, "viewer").await?;
    let group = insert_group(&harness.pool, &[author, viewer]).await?;
    let messaging = MessagingService::new(
        Arc::new(SqlxTransactionManager::new(harness.pool.clone())),
        Arc::new(PostgresMessagingRepository::new(harness.pool.clone())),
    );
    Ok(Fixture {
        author,
        viewer,
        group,
        messaging,
    })
}

fn identity(user_id: Uuid) -> AccessIdentity {
    AccessIdentity::new(user_id, Uuid::nil(), "admin-cli-test")
}

/// Sends a real message (so it has the `message.created` event, outbox row and notifications)
/// with one bound image attachment.
async fn send_message_with_media(
    harness: &AdminHarness,
    fixture: &Fixture,
    body: &str,
) -> TestResult<(Uuid, String)> {
    let outcome = fixture
        .messaging
        .send_message(
            &identity(fixture.author),
            SendMessageInput {
                chatroom_id: fixture.group.chatroom_id,
                client_msg_id: Uuid::new_v4(),
                body: Some(body.to_owned()),
                media_upload_ids: Vec::new(),
                idempotency_key: None,
            },
        )
        .await?;
    let message_id = match outcome {
        SendMessageOutcome::Created(message) | SendMessageOutcome::Existing(message) => message.id,
    };
    let object_key = insert_attachment(&harness.pool, fixture, message_id).await?;
    Ok((message_id, object_key))
}

async fn insert_attachment(
    pool: &PgPool,
    fixture: &Fixture,
    message_id: Uuid,
) -> TestResult<String> {
    let upload_id = Uuid::new_v4();
    let now = OffsetDateTime::now_utc();
    let object_key = format!("chat/{}/{}-0", fixture.group.chatroom_id, upload_id);
    sqlx::query(
        "INSERT INTO media_uploads \
             (id, user_id, object_key, scope, target_id, content_type, byte_size, \
              status, bound_message_id, confirmed_at, consumed_at, expires_at, created_at) \
         VALUES ($1, $2, $3, 'chat', $4, 'image/png', 4096, \
                 'bound', $5, $6, $7, $8, $9)",
    )
    .bind(upload_id)
    .bind(fixture.author)
    .bind(&object_key)
    .bind(fixture.group.chatroom_id)
    .bind(message_id)
    .bind(now - time::Duration::minutes(3))
    .bind(now - time::Duration::minutes(2))
    .bind(now + time::Duration::hours(1))
    .bind(now - time::Duration::minutes(4))
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO message_media \
             (id, message_id, media_upload_id, type, object_key, width, height, byte_size, \
              duration, position, filename, created_at) \
         VALUES ($1, $2, $3, 'image/png', $4, 320, 240, 4096, NULL, 0, 'media-0.bin', $5)",
    )
    .bind(Uuid::new_v4())
    .bind(message_id)
    .bind(upload_id)
    .bind(&object_key)
    .bind(now - time::Duration::minutes(2))
    .execute(pool)
    .await?;
    Ok(object_key)
}

/// The observable effects of a delete on one message, independent of who deleted it.
#[derive(Debug, Eq, PartialEq)]
struct DeleteEffects {
    message_deleted: bool,
    body_cleared: bool,
    media_rows: i64,
    upload_marked_deleted: bool,
    cleanup_intents: i64,
    created_event_scrubbed: bool,
    deleted_events: i64,
    deleted_outbox_events: i64,
    deleted_event_keys: Vec<String>,
}

async fn effects(pool: &PgPool, message_id: Uuid, object_key: &str) -> TestResult<DeleteEffects> {
    let (message_deleted, body_cleared) = sqlx::query_as::<_, (bool, bool)>(
        "SELECT deleted_at IS NOT NULL, body IS NULL FROM messages WHERE id = $1",
    )
    .bind(message_id)
    .fetch_one(pool)
    .await?;
    let media_rows = count(
        pool,
        "SELECT count(*) FROM message_media WHERE message_id = $1",
        message_id,
    )
    .await?;
    let upload_marked_deleted = sqlx::query_scalar::<_, bool>(
        "SELECT bool_and(deleted_at IS NOT NULL) FROM media_uploads WHERE bound_message_id = $1",
    )
    .bind(message_id)
    .fetch_one(pool)
    .await?;
    let cleanup_intents = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM account_object_deletion_intents WHERE object_key = $1",
    )
    .bind(object_key)
    .fetch_one(pool)
    .await?;
    let created_payload: Value = sqlx::query_scalar(
        "SELECT payload FROM conversation_events \
         WHERE event_type = 'message.created' AND payload ->> 'id' = $1::uuid::text",
    )
    .bind(message_id)
    .fetch_one(pool)
    .await?;
    let created_event_scrubbed =
        created_payload["body"].is_null() && created_payload["media"] == serde_json::json!([]);
    let deleted_events = sqlx::query_as::<_, (Uuid, Value)>(
        "SELECT id, payload FROM conversation_events \
         WHERE event_type = 'message.deleted' AND payload ->> 'message_id' = $1::uuid::text",
    )
    .bind(message_id)
    .fetch_all(pool)
    .await?;
    let mut deleted_outbox_events = 0;
    for (event_id, _) in &deleted_events {
        deleted_outbox_events += sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM outbox_events \
             WHERE event_type = 'message.deleted' AND conversation_event_id = $1",
        )
        .bind(event_id)
        .fetch_one(pool)
        .await?;
    }
    let mut deleted_event_keys = deleted_events
        .first()
        .and_then(|(_, payload)| payload.as_object())
        .map(|object| object.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    deleted_event_keys.sort();
    Ok(DeleteEffects {
        message_deleted,
        body_cleared,
        media_rows,
        upload_marked_deleted,
        cleanup_intents,
        created_event_scrubbed,
        deleted_events: i64::try_from(deleted_events.len())?,
        deleted_outbox_events,
        deleted_event_keys,
    })
}

#[tokio::test]
async fn the_moderator_delete_has_the_same_effects_as_an_author_delete() -> TestResult {
    let harness = AdminHarness::new().await?;
    let result: TestResult = async {
        let fixture = fixture(&harness).await?;
        let (by_moderator, moderator_key) =
            send_message_with_media(&harness, &fixture, BODY).await?;
        let (by_author, author_key) = send_message_with_media(&harness, &fixture, BODY).await?;

        let output = harness
            .run(&["messages", "delete", &by_moderator.to_string()])
            .await?;
        assert!(output.contains("deleted"), "{output}");
        assert!(output.contains(&fixture.group.chatroom_id.to_string()));
        fixture
            .messaging
            .delete_message(
                &identity(fixture.author),
                DeleteMessageInput {
                    chatroom_id: fixture.group.chatroom_id,
                    message_id: by_author,
                },
            )
            .await?;

        let moderated = effects(&harness.pool, by_moderator, &moderator_key).await?;
        let authored = effects(&harness.pool, by_author, &author_key).await?;
        assert_eq!(
            moderated,
            DeleteEffects {
                message_deleted: true,
                body_cleared: true,
                media_rows: 0,
                upload_marked_deleted: true,
                cleanup_intents: 1,
                created_event_scrubbed: true,
                deleted_events: 1,
                deleted_outbox_events: 1,
                deleted_event_keys: authored.deleted_event_keys.clone(),
            }
        );
        assert_eq!(
            moderated, authored,
            "the two deletes must look the same to sync"
        );

        // The delete event names the operator action without an account actor.
        let payload: Value = sqlx::query_scalar(
            "SELECT payload FROM conversation_events \
             WHERE event_type = 'message.deleted' AND payload ->> 'message_id' = $1::uuid::text",
        )
        .bind(by_moderator)
        .fetch_one(&harness.pool)
        .await?;
        assert_eq!(payload["reason"], "moderator_deleted");
        assert_eq!(payload["deleted_by"], Uuid::nil().to_string());
        assert_eq!(
            payload["chatroom_id"],
            fixture.group.chatroom_id.to_string()
        );
        assert_eq!(payload["group_id"], fixture.group.group_id.to_string());

        // The sync view: a member's delta page carries the delete event for the message and
        // never the original text.
        let page = fixture
            .messaging
            .events(
                &identity(fixture.viewer),
                DeltaInput {
                    conversation_id: fixture.group.chatroom_id,
                    after: None,
                    limit: 50,
                    contract_version: "2".to_owned(),
                },
            )
            .await?;
        assert!(page.items.iter().any(|item| matches!(
            item,
            DeltaItem::MessageDeleted(event)
                if event.data.message_id == by_moderator
                    && event.data.reason == "moderator_deleted"
        )));
        assert!(
            !serde_json::to_string(&page)?.contains(BODY),
            "the deleted text is still visible in the delta feed"
        );
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn the_delete_is_idempotent_and_actions_every_open_report_on_the_message() -> TestResult {
    let harness = AdminHarness::new().await?;
    let result: TestResult = async {
        let fixture = fixture(&harness).await?;
        let (message_id, object_key) = send_message_with_media(&harness, &fixture, BODY).await?;
        let (other_message, _) = send_message_with_media(&harness, &fixture, "unrelated").await?;
        let group_id = fixture.group.group_id;
        let first = ReportFixture::message(fixture.viewer, group_id, message_id, BODY)
            .insert(&harness.pool)
            .await?;
        let second = ReportFixture::message(fixture.viewer, group_id, message_id, BODY)
            .insert(&harness.pool)
            .await?;
        let dismissed = ReportFixture::message(fixture.viewer, group_id, message_id, BODY)
            .handled("dismissed", 1)
            .insert(&harness.pool)
            .await?;
        let unrelated =
            ReportFixture::message(fixture.viewer, group_id, other_message, "unrelated")
                .insert(&harness.pool)
                .await?;

        let output = harness
            .run(&["messages", "delete", &message_id.to_string()])
            .await?;
        assert!(output.contains("2 open report(s) actioned"), "{output}");
        assert!(output.contains(&first.to_string()) && output.contains(&second.to_string()));
        for report in [first, second] {
            assert_eq!(
                report_status(&harness.pool, report).await?,
                ("actioned".to_owned(), true)
            );
        }
        assert_eq!(
            report_status(&harness.pool, dismissed).await?.0,
            "dismissed"
        );
        assert_eq!(report_status(&harness.pool, unrelated).await?.0, "open");
        let once = effects(&harness.pool, message_id, &object_key).await?;

        // A repeat succeeds without a second delete event or cleanup intent.
        let repeat = harness
            .run(&["messages", "delete", &message_id.to_string()])
            .await?;
        assert!(repeat.contains("already deleted"), "{repeat}");
        assert!(repeat.contains("0 open report(s) actioned"));
        assert_eq!(effects(&harness.pool, message_id, &object_key).await?, once);
        assert_eq!(once.deleted_events, 1);
        assert_eq!(once.cleanup_intents, 1);

        // A report filed before an author deleted the message is actioned by the operator run.
        let (authored, _) = send_message_with_media(&harness, &fixture, BODY).await?;
        let late = ReportFixture::message(fixture.viewer, group_id, authored, BODY)
            .insert(&harness.pool)
            .await?;
        fixture
            .messaging
            .delete_message(
                &identity(fixture.author),
                DeleteMessageInput {
                    chatroom_id: fixture.group.chatroom_id,
                    message_id: authored,
                },
            )
            .await?;
        let output = harness
            .run(&["messages", "delete", &authored.to_string()])
            .await?;
        assert!(output.contains("already deleted") && output.contains("1 open report(s) actioned"));
        assert_eq!(report_status(&harness.pool, late).await?.0, "actioned");
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn unknown_system_and_malformed_ids_are_refused_without_changes() -> TestResult {
    let harness = AdminHarness::new().await?;
    let result: TestResult = async {
        let fixture = fixture(&harness).await?;
        let events_before = count(
            &harness.pool,
            "SELECT count(*) FROM conversation_events WHERE conversation_id = $1",
            fixture.group.chatroom_id,
        )
        .await?;

        assert_failure(
            harness
                .run(&["messages", "delete", &Uuid::new_v4().to_string()])
                .await,
            "not found",
        )?;

        let system_message = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO messages (id, chatroom_id, body, type) VALUES ($1, $2, $3, 'system')",
        )
        .bind(system_message)
        .bind(fixture.group.chatroom_id)
        .bind("someone joined")
        .execute(&harness.pool)
        .await?;
        assert_failure(
            harness
                .run(&["messages", "delete", &system_message.to_string()])
                .await,
            "not a user message",
        )?;
        let still_live = sqlx::query_scalar::<_, bool>(
            "SELECT deleted_at IS NULL AND body IS NOT NULL FROM messages WHERE id = $1",
        )
        .bind(system_message)
        .fetch_one(&harness.pool)
        .await?;
        assert!(still_live, "a system message must not be moderated");

        assert_usage(harness.run(&["messages", "delete", "not-a-uuid"]).await)?;
        assert_usage(harness.run(&["messages", "delete"]).await)?;
        assert_eq!(
            count(
                &harness.pool,
                "SELECT count(*) FROM conversation_events WHERE conversation_id = $1",
                fixture.group.chatroom_id
            )
            .await?,
            events_before,
            "a refused delete must not append events"
        );
        Ok(())
    }
    .await;
    harness.finish(result).await
}
