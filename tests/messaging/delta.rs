use std::collections::HashSet;

use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    messaging_helpers::{TestApp, assert_error, json_body},
    messaging_http::message_payload,
    postgres_support::TestResult,
};

#[tokio::test]
async fn s1_pages_strictly_forward_across_commits_and_an_unknown_marker() -> TestResult {
    let app = TestApp::new().await?;
    for index in 1..=5 {
        send_known(&app, &format!("message-{index}")).await?;
    }

    let first = page(&app, None, "2").await?;
    assert_page(&first, &["1", "2"], Some("2"))?;

    let unknown_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO conversation_events \
         (id, conversation_id, event_type, event_version, payload) \
         VALUES ($1, $2, 'future.event', 1, $3)",
    )
    .bind(unknown_id)
    .bind(app.fixture.chatroom_id)
    .bind(json!({"reconcile_scope": "chat_history"}))
    .execute(&app.pool)
    .await?;
    send_known(&app, "after-marker").await?;

    let second = page(&app, Some("2"), "2").await?;
    let third = page(&app, Some("4"), "2").await?;
    let fourth = page(&app, Some("6"), "2").await?;
    let terminal = page(&app, Some("7"), "2").await?;
    assert_page(&second, &["3", "4"], Some("4"))?;
    assert_page(&third, &["5", "6"], Some("6"))?;
    assert_eq!(third["items"][1]["event_id"], unknown_id.to_string());
    assert_eq!(third["items"][1]["reconcile_scope"], "chat_history");
    assert_page(&fourth, &["7"], None)?;
    assert_page(&terminal, &[], None)?;

    let mut observer = Observer::default();
    for page in [&first, &second, &third, &fourth, &terminal] {
        observer.apply(page)?;
    }
    assert_eq!(observer.last_cursor, Some(7));
    assert_eq!(observer.seen_event_ids.len(), 7);

    let previous = page(&app, None, "1").await?;
    assert_page(&previous, &["1", "2"], Some("2"))?;
    app.dispose().await
}

#[tokio::test]
async fn s1_rejects_unsupported_versions_and_unsafe_unknown_projection() -> TestResult {
    let app = TestApp::new().await?;
    for version in ["0", "999"] {
        let unsupported = app
            .events(
                Some(&app.fixture.access_token),
                app.fixture.chatroom_id,
                None,
                2,
                Some(version),
            )
            .await?;
        assert_error(
            unsupported,
            StatusCode::UPGRADE_REQUIRED,
            "contract_upgrade_required",
        )
        .await?;
    }

    sqlx::query(
        "INSERT INTO conversation_events \
         (id, conversation_id, event_type, event_version, payload) \
         VALUES ($1, $2, 'future.event', 1, '{}'::jsonb)",
    )
    .bind(Uuid::new_v4())
    .bind(app.fixture.chatroom_id)
    .execute(&app.pool)
    .await?;
    let unsafe_projection = app
        .events(
            Some(&app.fixture.access_token),
            app.fixture.chatroom_id,
            None,
            2,
            Some("2"),
        )
        .await?;
    assert_error(
        unsafe_projection,
        StatusCode::UPGRADE_REQUIRED,
        "contract_upgrade_required",
    )
    .await?;
    app.dispose().await
}

#[tokio::test]
async fn s1_requires_bearer_authentication_and_current_membership() -> TestResult {
    let app = TestApp::new().await?;
    for token in [None, Some("invalid-token")] {
        let response = app
            .events(token, app.fixture.chatroom_id, None, 2, Some("2"))
            .await?;
        assert_error(
            response,
            StatusCode::UNAUTHORIZED,
            "authentication_required",
        )
        .await?;
    }

    let outsider = app.seed_another().await?;
    let response = app
        .events(
            Some(&outsider.access_token),
            app.fixture.chatroom_id,
            None,
            2,
            Some("2"),
        )
        .await?;
    assert_error(response, StatusCode::FORBIDDEN, "membership_required").await?;
    app.dispose().await
}

#[tokio::test]
async fn s1_v2_reads_topic_deleted_from_main_feed_and_v1_keeps_marker() -> TestResult {
    let app = TestApp::new().await?;
    let topic_id = Uuid::new_v4();
    let topic_chatroom_id = Uuid::new_v4();
    seed_topic_room(&app, topic_id, topic_chatroom_id).await?;
    let event_id = Uuid::new_v4();
    let announcement_message_id = Uuid::new_v4();
    insert_topic_deleted_event(
        &app,
        app.fixture.chatroom_id,
        event_id,
        topic_id,
        topic_chatroom_id,
        app.fixture.group_id,
        Some(announcement_message_id),
    )
    .await?;

    let current = page_for(&app, app.fixture.chatroom_id, None, 10, "2").await?;
    let item = &current["items"][0];
    assert_eq!(item["type"], "topic.deleted");
    assert_eq!(item["event_id"], event_id.to_string());
    assert_eq!(item["conversation_id"], app.fixture.chatroom_id.to_string());
    assert_eq!(item["data"]["topic_id"], topic_id.to_string());
    assert_eq!(
        item["data"]["topic_chatroom_id"],
        topic_chatroom_id.to_string()
    );
    assert_eq!(
        item["data"]["announcement_message_id"],
        announcement_message_id.to_string()
    );

    let previous = page_for(&app, app.fixture.chatroom_id, None, 10, "1").await?;
    let marker = &previous["items"][0];
    assert_eq!(marker["event_id"], event_id.to_string());
    assert_eq!(marker["reconcile_scope"], "group_topics");
    assert!(marker.get("type").is_none());
    assert!(marker.get("data").is_none());
    app.dispose().await
}

#[tokio::test]
async fn s1_v2_does_not_type_topic_deleted_from_wrong_group_or_topic_feed() -> TestResult {
    let app = TestApp::new().await?;
    let topic_id = Uuid::new_v4();
    let topic_chatroom_id = Uuid::new_v4();
    seed_topic_room(&app, topic_id, topic_chatroom_id).await?;
    let other_main_chatroom_id = seed_other_main_room(&app).await?;

    let wrong_group_event_id = Uuid::new_v4();
    insert_topic_deleted_event(
        &app,
        other_main_chatroom_id,
        wrong_group_event_id,
        topic_id,
        topic_chatroom_id,
        app.fixture.group_id,
        None,
    )
    .await?;
    let wrong_group = page_for(&app, other_main_chatroom_id, None, 10, "2").await?;
    let wrong_group_marker = &wrong_group["items"][0];
    assert_eq!(
        wrong_group_marker["event_id"],
        wrong_group_event_id.to_string()
    );
    assert_eq!(wrong_group_marker["reconcile_scope"], "group_topics");
    assert!(wrong_group_marker.get("type").is_none());
    assert!(wrong_group_marker.get("data").is_none());

    let topic_feed_event_id = Uuid::new_v4();
    insert_topic_deleted_event(
        &app,
        topic_chatroom_id,
        topic_feed_event_id,
        topic_id,
        topic_chatroom_id,
        app.fixture.group_id,
        None,
    )
    .await?;
    let topic_feed = page_for(&app, topic_chatroom_id, None, 10, "2").await?;
    let topic_feed_marker = &topic_feed["items"][0];
    assert_eq!(
        topic_feed_marker["event_id"],
        topic_feed_event_id.to_string()
    );
    assert_eq!(topic_feed_marker["reconcile_scope"], "group_topics");
    assert!(topic_feed_marker.get("type").is_none());
    assert!(topic_feed_marker.get("data").is_none());
    app.dispose().await
}

async fn send_known(app: &TestApp, body: &str) -> TestResult {
    let response = app
        .send(
            Some(&app.fixture.access_token),
            app.fixture.chatroom_id,
            message_payload(Uuid::new_v4(), body),
            None,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    Ok(())
}

async fn page(app: &TestApp, after: Option<&str>, version: &str) -> TestResult<Value> {
    page_for(app, app.fixture.chatroom_id, after, 2, version).await
}

async fn page_for(
    app: &TestApp,
    conversation_id: Uuid,
    after: Option<&str>,
    limit: u32,
    version: &str,
) -> TestResult<Value> {
    let response = app
        .events(
            Some(&app.fixture.access_token),
            conversation_id,
            after,
            limit,
            Some(version),
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-jamye-contract-version")
            .and_then(|header| header.to_str().ok()),
        Some(version)
    );
    json_body(response).await
}

async fn seed_topic_room(app: &TestApp, topic_id: Uuid, topic_chatroom_id: Uuid) -> TestResult {
    sqlx::query(
        "INSERT INTO topics \
             (id, group_id, author_id, idempotency_key, request_fingerprint, title) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(topic_id)
    .bind(app.fixture.group_id)
    .bind(app.fixture.user_id)
    .bind(Uuid::new_v4())
    .bind("a".repeat(64))
    .bind("delta topic")
    .execute(&app.pool)
    .await?;
    sqlx::query(
        "INSERT INTO chatrooms (id, group_id, type, topic_id) VALUES ($1, $2, 'topic', $3)",
    )
    .bind(topic_chatroom_id)
    .bind(app.fixture.group_id)
    .bind(topic_id)
    .execute(&app.pool)
    .await?;
    Ok(())
}

async fn seed_other_main_room(app: &TestApp) -> TestResult<Uuid> {
    let group_id = Uuid::new_v4();
    let chatroom_id = Uuid::new_v4();
    sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, $2, $3)")
        .bind(group_id)
        .bind("delta other group")
        .bind(app.fixture.user_id)
        .execute(&app.pool)
        .await?;
    sqlx::query(
        "INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, 'owner')",
    )
    .bind(Uuid::new_v4())
    .bind(group_id)
    .bind(app.fixture.user_id)
    .execute(&app.pool)
    .await?;
    sqlx::query(
        "INSERT INTO chatrooms (id, group_id, type, topic_id) VALUES ($1, $2, 'main', NULL)",
    )
    .bind(chatroom_id)
    .bind(group_id)
    .execute(&app.pool)
    .await?;
    Ok(chatroom_id)
}

async fn insert_topic_deleted_event(
    app: &TestApp,
    conversation_id: Uuid,
    event_id: Uuid,
    topic_id: Uuid,
    topic_chatroom_id: Uuid,
    group_id: Uuid,
    announcement_message_id: Option<Uuid>,
) -> TestResult {
    sqlx::query(
        "INSERT INTO conversation_events \
             (id, conversation_id, event_type, event_version, payload) \
         VALUES ($1, $2, 'topic.deleted', 1, $3)",
    )
    .bind(event_id)
    .bind(conversation_id)
    .bind(json!({
        "topic_id": topic_id,
        "topic_chatroom_id": topic_chatroom_id,
        "group_id": group_id,
        "deleted_at": "2026-09-29T00:00:00Z",
        "deleted_by": app.fixture.user_id,
        "announcement_message_id": announcement_message_id,
    }))
    .execute(&app.pool)
    .await?;
    Ok(())
}

fn assert_page(page: &Value, cursors: &[&str], next_cursor: Option<&str>) -> TestResult {
    let actual = page["items"]
        .as_array()
        .ok_or("items must be an array")?
        .iter()
        .map(|item| item["cursor"].as_str().ok_or("cursor must be a string"))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(actual, cursors);
    assert_eq!(page["next_cursor"].as_str(), next_cursor);
    if next_cursor.is_none() {
        assert!(page["next_cursor"].is_null());
    }
    Ok(())
}

#[derive(Default)]
struct Observer {
    seen_event_ids: HashSet<String>,
    last_cursor: Option<i64>,
}

impl Observer {
    fn apply(&mut self, page: &Value) -> TestResult {
        for item in page["items"].as_array().ok_or("items must be an array")? {
            let event_id = item["event_id"]
                .as_str()
                .ok_or("event_id must be a string")?;
            let cursor = item["cursor"]
                .as_str()
                .ok_or("cursor must be a string")?
                .parse::<i64>()?;
            assert!(self.seen_event_ids.insert(event_id.to_owned()));
            assert!(self.last_cursor.is_none_or(|previous| cursor > previous));
            self.last_cursor = Some(cursor);
        }
        Ok(())
    }
}
