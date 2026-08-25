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

    let first = page(&app, None, "1").await?;
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

    let second = page(&app, Some("2"), "1").await?;
    let third = page(&app, Some("4"), "1").await?;
    let fourth = page(&app, Some("6"), "1").await?;
    let terminal = page(&app, Some("7"), "1").await?;
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

    let previous = page(&app, None, "0").await?;
    assert_page(&previous, &["1", "2"], Some("2"))?;
    app.dispose().await
}

#[tokio::test]
async fn s1_rejects_unsupported_versions_and_unsafe_unknown_projection() -> TestResult {
    let app = TestApp::new().await?;
    let unsupported = app
        .events(
            Some(&app.fixture.access_token),
            app.fixture.chatroom_id,
            None,
            2,
            Some("999"),
        )
        .await?;
    assert_error(
        unsupported,
        StatusCode::UPGRADE_REQUIRED,
        "contract_upgrade_required",
    )
    .await?;

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
            Some("1"),
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
            .events(token, app.fixture.chatroom_id, None, 2, Some("1"))
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
            Some("1"),
        )
        .await?;
    assert_error(response, StatusCode::FORBIDDEN, "membership_required").await?;
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
    let response = app
        .events(
            Some(&app.fixture.access_token),
            app.fixture.chatroom_id,
            after,
            2,
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
