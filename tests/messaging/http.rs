use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    messaging_helpers::{TestApp, assert_error, counts, json_body},
    postgres_support::TestResult,
};

#[tokio::test]
async fn c4_preserves_content_idempotency_and_exact_text() -> TestResult {
    let app = TestApp::new().await?;
    let token = app.fixture.access_token.as_str();
    let chatroom_id = app.fixture.chatroom_id;

    for payload in [
        json!({"body": "missing client message ID"}),
        json!({"client_msg_id": Uuid::new_v4()}),
        json!({"client_msg_id": Uuid::new_v4(), "body": null, "media": []}),
        json!({"client_msg_id": Uuid::new_v4(), "body": ""}),
    ] {
        let expected = if payload.get("client_msg_id").is_some() {
            "message_content_required"
        } else {
            "request_validation_failed"
        };
        let response = app.send(Some(token), chatroom_id, payload, None).await?;
        assert_error(response, StatusCode::UNPROCESSABLE_ENTITY, expected).await?;
    }

    let response = app
        .send(
            Some(token),
            chatroom_id,
            json!({
                "client_msg_id": Uuid::new_v4(),
                "media": [{"media_upload_id": Uuid::new_v4()}]
            }),
            None,
        )
        .await?;
    assert_error(
        response,
        StatusCode::UNPROCESSABLE_ENTITY,
        "media_not_available",
    )
    .await?;

    let client_msg_id = Uuid::new_v4();
    let response = app
        .send(
            Some(token),
            chatroom_id,
            message_payload(client_msg_id, "   "),
            Some(&Uuid::new_v4().to_string()),
        )
        .await?;
    assert_error(
        response,
        StatusCode::UNPROCESSABLE_ENTITY,
        "idempotency_key_mismatch",
    )
    .await?;

    let created = app
        .send(
            Some(token),
            chatroom_id,
            message_payload(client_msg_id, "   "),
            None,
        )
        .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let canonical = json_body(created).await?;
    assert_eq!(canonical["body"], "   ");
    assert_eq!(canonical["media"], json!([]));

    for header in [None, Some(client_msg_id.to_string())] {
        let retry = app
            .send(
                Some(token),
                chatroom_id,
                message_payload(client_msg_id, "   "),
                header.as_deref(),
            )
            .await?;
        assert_eq!(retry.status(), StatusCode::OK);
        assert_eq!(json_body(retry).await?, canonical);
    }

    let conflict = app
        .send(
            Some(token),
            chatroom_id,
            message_payload(client_msg_id, "changed"),
            None,
        )
        .await?;
    assert_error(conflict, StatusCode::CONFLICT, "idempotency_conflict").await?;
    assert_eq!(counts(&app.pool).await?, (1, 1, 1));
    let (event_id, cursor, event_payload) = sqlx::query_as::<_, (Uuid, i64, Value)>(
        "SELECT id, cursor, payload FROM conversation_events",
    )
    .fetch_one(&app.pool)
    .await?;
    assert_eq!(event_payload, canonical);
    let (linked_event_id, outbox_payload) = sqlx::query_as::<_, (Option<Uuid>, Value)>(
        "SELECT conversation_event_id, payload FROM outbox_events",
    )
    .fetch_one(&app.pool)
    .await?;
    assert_eq!(linked_event_id, Some(event_id));
    assert_eq!(outbox_payload["event_id"], event_id.to_string());
    assert_eq!(outbox_payload["cursor"], cursor.to_string());
    assert_eq!(outbox_payload["data"], canonical);
    app.dispose().await
}

#[tokio::test]
async fn c4_matching_header_concurrent_retries_share_one_canonical_commit() -> TestResult {
    let app = TestApp::new().await?;
    let client_msg_id = Uuid::new_v4();
    let idempotency_key = client_msg_id.to_string();
    let first = app.send(
        Some(&app.fixture.access_token),
        app.fixture.chatroom_id,
        message_payload(client_msg_id, "concurrent"),
        Some(&idempotency_key),
    );
    let second = app.send(
        Some(&app.fixture.access_token),
        app.fixture.chatroom_id,
        message_payload(client_msg_id, "concurrent"),
        Some(&idempotency_key),
    );
    let (first, second) = tokio::join!(first, second);
    let first = first?;
    let second = second?;
    let statuses = [first.status(), second.status()];
    assert!(statuses.contains(&StatusCode::CREATED));
    assert!(statuses.contains(&StatusCode::OK));
    assert_eq!(json_body(first).await?, json_body(second).await?);
    assert_eq!(counts(&app.pool).await?, (1, 1, 1));
    app.dispose().await
}

#[tokio::test]
async fn c6_deletes_author_messages_scrubs_content_and_is_idempotent() -> TestResult {
    let app = TestApp::new().await?;
    let token = app.fixture.access_token.as_str();
    let chatroom_id = app.fixture.chatroom_id;

    let denied = app
        .send(
            Some(token),
            chatroom_id,
            message_payload(Uuid::new_v4(), "not yours"),
            None,
        )
        .await?;
    assert_eq!(denied.status(), StatusCode::CREATED);
    let denied_id = uuid_field(&json_body(denied).await?, "id")?;
    let other_user_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, '다른 작성자')")
        .bind(other_user_id)
        .execute(&app.pool)
        .await?;
    sqlx::query(
        "INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, 'member')",
    )
    .bind(Uuid::new_v4())
    .bind(app.fixture.group_id)
    .bind(other_user_id)
    .execute(&app.pool)
    .await?;
    sqlx::query("UPDATE messages SET sender_id = $2 WHERE id = $1")
        .bind(denied_id)
        .bind(other_user_id)
        .execute(&app.pool)
        .await?;
    let forbidden = app
        .delete_message(Some(token), chatroom_id, denied_id)
        .await?;
    assert_error(forbidden, StatusCode::FORBIDDEN, "message_author_required").await?;
    sqlx::query("UPDATE messages SET deleted_at = clock_timestamp() WHERE id = $1")
        .bind(denied_id)
        .execute(&app.pool)
        .await?;
    let hidden = app
        .delete_message(Some(token), chatroom_id, denied_id)
        .await?;
    assert_error(hidden, StatusCode::NOT_FOUND, "message_not_found").await?;

    let client_msg_id = Uuid::new_v4();
    let created = app
        .send(
            Some(token),
            chatroom_id,
            message_payload(client_msg_id, "delete me"),
            None,
        )
        .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let message_id = uuid_field(&json_body(created).await?, "id")?;

    let deleted = app
        .delete_message(Some(token), chatroom_id, message_id)
        .await?;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    let retry = app
        .delete_message(Some(token), chatroom_id, message_id)
        .await?;
    assert_eq!(retry.status(), StatusCode::NO_CONTENT);
    let resend = app
        .send(
            Some(token),
            chatroom_id,
            message_payload(client_msg_id, "delete me"),
            None,
        )
        .await?;
    assert_error(resend, StatusCode::CONFLICT, "idempotency_conflict").await?;

    let (body, deleted_present) = sqlx::query_as::<_, (Option<String>, bool)>(
        "SELECT body, deleted_at IS NOT NULL FROM messages WHERE id = $1",
    )
    .bind(message_id)
    .fetch_one(&app.pool)
    .await?;
    assert_eq!(body, None);
    assert!(deleted_present);

    let created_payload: Value = sqlx::query_scalar(
        "SELECT payload FROM conversation_events \
         WHERE event_type = 'message.created' AND payload ->> 'id' = $1::uuid::text",
    )
    .bind(message_id)
    .fetch_one(&app.pool)
    .await?;
    assert_eq!(created_payload["body"], Value::Null);
    assert_eq!(created_payload["media"], json!([]));

    let (deleted_event_id, deleted_event_payload) = sqlx::query_as::<_, (Uuid, Value)>(
        "SELECT id, payload FROM conversation_events \
         WHERE event_type = 'message.deleted' AND payload ->> 'message_id' = $1::uuid::text",
    )
    .bind(message_id)
    .fetch_one(&app.pool)
    .await?;
    assert_eq!(
        deleted_event_payload["chatroom_id"],
        chatroom_id.to_string()
    );
    assert_eq!(
        deleted_event_payload["deleted_by"],
        app.fixture.user_id.to_string()
    );
    assert_eq!(deleted_event_payload["reason"], "author_deleted");

    let outbox_payload: Value = sqlx::query_scalar(
        "SELECT payload FROM outbox_events \
         WHERE event_type = 'message.deleted' AND conversation_event_id = $1",
    )
    .bind(deleted_event_id)
    .fetch_one(&app.pool)
    .await?;
    assert_eq!(outbox_payload["type"], "message.deleted");
    assert_eq!(outbox_payload["data"]["message_id"], message_id.to_string());

    let v2 = app
        .events(Some(token), chatroom_id, None, 10, Some("2"))
        .await?;
    assert_eq!(v2.status(), StatusCode::OK);
    let v2 = json_body(v2).await?;
    assert!(v2["items"].as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item["type"] == "message.deleted"
                && item["event_id"] == deleted_event_id.to_string()
                && item["data"]["message_id"] == message_id.to_string()
        })
    }));
    let v1 = app
        .events(Some(token), chatroom_id, None, 10, Some("1"))
        .await?;
    assert_eq!(v1.status(), StatusCode::OK);
    let v1 = json_body(v1).await?;
    assert!(v1["items"].as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item["event_id"] == deleted_event_id.to_string()
                && item["reconcile_scope"] == "chat_history"
                && item.get("type").is_none()
        })
    }));

    app.dispose().await
}

#[tokio::test]
async fn c4_auth_and_membership_fail_without_resource_disclosure() -> TestResult {
    let app = TestApp::new().await?;
    let payload = message_payload(Uuid::new_v4(), "private");

    let missing_auth = app
        .send(None, app.fixture.chatroom_id, payload.clone(), None)
        .await?;
    assert_error(
        missing_auth,
        StatusCode::UNAUTHORIZED,
        "authentication_required",
    )
    .await?;

    let invalid_auth = app
        .send(
            Some("invalid-token"),
            app.fixture.chatroom_id,
            payload.clone(),
            None,
        )
        .await?;
    assert_error(
        invalid_auth,
        StatusCode::UNAUTHORIZED,
        "authentication_required",
    )
    .await?;

    let outsider = app.seed_another().await?;
    for chatroom_id in [app.fixture.chatroom_id, Uuid::new_v4()] {
        let denied = app
            .send(
                Some(&outsider.access_token),
                chatroom_id,
                payload.clone(),
                None,
            )
            .await?;
        assert_error(denied, StatusCode::FORBIDDEN, "membership_required").await?;
    }

    sqlx::query("UPDATE groups SET deleted_at = clock_timestamp() WHERE id = $1")
        .bind(app.fixture.group_id)
        .execute(&app.pool)
        .await?;
    let deleted = app
        .send(
            Some(&app.fixture.access_token),
            app.fixture.chatroom_id,
            payload,
            None,
        )
        .await?;
    assert_error(deleted, StatusCode::FORBIDDEN, "membership_required").await?;
    assert_eq!(counts(&app.pool).await?, (0, 0, 0));
    app.dispose().await
}

#[tokio::test]
async fn a_failure_after_message_insert_rolls_back_the_entire_command() -> TestResult {
    let app = TestApp::new().await?;
    sqlx::query(
        "CREATE FUNCTION reject_task_4a_event() RETURNS trigger AS $$ \
         BEGIN RAISE EXCEPTION 'forced task-4a rollback'; END; \
         $$ LANGUAGE plpgsql",
    )
    .execute(&app.pool)
    .await?;
    sqlx::query(
        "CREATE TRIGGER reject_task_4a_event \
         BEFORE INSERT ON conversation_events \
         FOR EACH ROW EXECUTE FUNCTION reject_task_4a_event()",
    )
    .execute(&app.pool)
    .await?;

    let response = app
        .send(
            Some(&app.fixture.access_token),
            app.fixture.chatroom_id,
            message_payload(Uuid::new_v4(), "rollback"),
            None,
        )
        .await?;
    assert_error(
        response,
        StatusCode::SERVICE_UNAVAILABLE,
        "database_unavailable",
    )
    .await?;
    assert_eq!(counts(&app.pool).await?, (0, 0, 0));
    app.dispose().await
}

pub(crate) fn message_payload(client_msg_id: Uuid, body: &str) -> Value {
    json!({"client_msg_id": client_msg_id, "body": body, "media": []})
}

fn uuid_field(value: &Value, field: &str) -> TestResult<Uuid> {
    Ok(Uuid::try_parse(value[field].as_str().ok_or_else(
        || format!("{field} must be a UUID string"),
    )?)?)
}
