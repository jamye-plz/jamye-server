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
