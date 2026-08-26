use std::{io, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode, header::AUTHORIZATION},
};
use jamye_server::transport::http::chatrooms::{ChatroomsHttpState, router as chatrooms_router};
use serde_json::{Value, json};
use time::OffsetDateTime;
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    TestResult,
    chatroom_helpers::{
        TestAccessVerifier, bearer, harness, insert_event, insert_user_message, topology,
    },
    postgres_support::TestDatabase,
};

#[tokio::test]
async fn c1_c2_and_c3_http_use_exact_authenticated_mobile_shapes() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let harness = harness(pool.clone());
    let message_id = insert_user_message(
        &pool,
        fixture.chatroom_id,
        fixture.owner_id,
        "정확한 히스토리",
        OffsetDateTime::UNIX_EPOCH + time::Duration::hours(4),
    )
    .await?;
    let cursor = insert_event(&pool, fixture.chatroom_id).await?;
    let router = chatrooms_router(ChatroomsHttpState::new(
        harness.service,
        Arc::new(TestAccessVerifier),
    ));

    let unauthorized = router
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!("/api/v1/groups/{}/chatrooms", fixture.group_id),
            None,
        )?)
        .await?;
    assert_error(
        unauthorized,
        StatusCode::UNAUTHORIZED,
        "authentication_required",
    )
    .await?;

    let chatrooms = router
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!("/api/v1/groups/{}/chatrooms?limit=10", fixture.group_id),
            Some(fixture.owner_id),
        )?)
        .await?;
    assert_eq!(chatrooms.status(), StatusCode::OK);
    let chatrooms = response_json(chatrooms).await?;
    assert_eq!(chatrooms["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(chatrooms["items"][0]["id"], fixture.chatroom_id.to_string());
    assert_eq!(chatrooms["items"][0]["type"], "main");
    assert_eq!(chatrooms["items"][0]["topic_id"], Value::Null);
    assert_eq!(chatrooms["next_cursor"], Value::Null);

    let messages = router
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!(
                "/api/v1/chatrooms/{}/messages?limit=10",
                fixture.chatroom_id
            ),
            Some(fixture.owner_id),
        )?)
        .await?;
    assert_eq!(messages.status(), StatusCode::OK);
    let messages = response_json(messages).await?;
    let item = &messages["items"][0];
    assert_eq!(item["id"], message_id.to_string());
    assert_eq!(item["sender_nickname"], "채팅방 소유자");
    assert_eq!(item["sender_avatar_url"], "https://cdn.test/owner.png");
    assert_eq!(item["body"], "정확한 히스토리");
    assert_eq!(item["type"], "user");
    assert_eq!(item["media"], json!([]));

    let read = router
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/api/v1/chatrooms/{}/read", fixture.chatroom_id),
            Some(fixture.owner_id),
            json!({"cursor": cursor.to_string()}),
        )?)
        .await?;
    assert_eq!(read.status(), StatusCode::OK);
    let read = response_json(read).await?;
    assert_eq!(read["chatroom_id"], fixture.chatroom_id.to_string());
    assert_eq!(read["last_read_cursor"], cursor.to_string());
    assert!(read["updated_at"].as_str().is_some());
    assert!(read.get("id").is_none());
    assert!(read.get("user_id").is_none());

    for request in [
        empty_request(
            "GET",
            &format!(
                "/api/v1/groups/{}/chatrooms?before={}",
                fixture.group_id, fixture.chatroom_id
            ),
            Some(fixture.owner_id),
        )?,
        empty_request(
            "GET",
            &format!("/api/v1/chatrooms/{}/messages?limit=0", fixture.chatroom_id),
            Some(fixture.owner_id),
        )?,
        json_request(
            "POST",
            &format!("/api/v1/chatrooms/{}/read", fixture.chatroom_id),
            Some(fixture.owner_id),
            json!({"cursor": cursor}),
        )?,
    ] {
        let response = router.clone().oneshot(request).await?;
        assert_error(
            response,
            StatusCode::UNPROCESSABLE_ENTITY,
            "request_validation_failed",
        )
        .await?;
    }

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn bola_failures_share_one_safe_membership_error_and_never_mutate() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let harness = harness(pool.clone());
    let secret_message = insert_user_message(
        &pool,
        fixture.chatroom_id,
        fixture.owner_id,
        "절대 노출되면 안 되는 본문",
        OffsetDateTime::UNIX_EPOCH + time::Duration::hours(5),
    )
    .await?;
    let owner_cursor = insert_event(&pool, fixture.chatroom_id).await?;
    let foreign_cursor = insert_event(&pool, fixture.other_chatroom_id).await?;
    let foreign_message = insert_user_message(
        &pool,
        fixture.other_chatroom_id,
        fixture.outsider_id,
        "다른 그룹 본문",
        OffsetDateTime::UNIX_EPOCH + time::Duration::hours(5),
    )
    .await?;
    let router = chatrooms_router(ChatroomsHttpState::new(
        harness.service,
        Arc::new(TestAccessVerifier),
    ));

    let denied_requests = [
        empty_request(
            "GET",
            &format!("/api/v1/groups/{}/chatrooms", fixture.group_id),
            Some(fixture.outsider_id),
        )?,
        empty_request(
            "GET",
            &format!("/api/v1/chatrooms/{}/messages", fixture.chatroom_id),
            Some(fixture.outsider_id),
        )?,
        json_request(
            "POST",
            &format!("/api/v1/chatrooms/{}/read", fixture.chatroom_id),
            Some(fixture.outsider_id),
            json!({"cursor": owner_cursor.to_string()}),
        )?,
        empty_request(
            "GET",
            &format!("/api/v1/chatrooms/{}/messages", Uuid::new_v4()),
            Some(fixture.owner_id),
        )?,
    ];
    for request in denied_requests {
        let response = router.clone().oneshot(request).await?;
        let body = assert_error(response, StatusCode::FORBIDDEN, "membership_required").await?;
        let serialized = serde_json::to_string(&body)?;
        for secret in [
            fixture.chatroom_id.to_string(),
            secret_message.to_string(),
            "채팅방 소유자".to_owned(),
            "절대 노출되면 안 되는 본문".to_owned(),
        ] {
            assert!(!serialized.contains(&secret));
        }
    }

    let cross_history = router
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!(
                "/api/v1/chatrooms/{}/messages?before={foreign_message}",
                fixture.chatroom_id
            ),
            Some(fixture.owner_id),
        )?)
        .await?;
    assert_error(
        cross_history,
        StatusCode::UNPROCESSABLE_ENTITY,
        "request_validation_failed",
    )
    .await?;

    let cross_read = router
        .oneshot(json_request(
            "POST",
            &format!("/api/v1/chatrooms/{}/read", fixture.chatroom_id),
            Some(fixture.owner_id),
            json!({"cursor": foreign_cursor.to_string()}),
        )?)
        .await?;
    assert_error(
        cross_read,
        StatusCode::UNPROCESSABLE_ENTITY,
        "request_validation_failed",
    )
    .await?;
    let marker_count: i64 = sqlx::query_scalar("SELECT count(*) FROM chatroom_reads")
        .fetch_one(&pool)
        .await?;
    assert_eq!(marker_count, 0);

    pool.close().await;
    database.dispose().await
}

fn json_request(
    method: &str,
    uri: &str,
    user_id: Option<Uuid>,
    body: Value,
) -> TestResult<Request<Body>> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(user_id) = user_id {
        builder = builder.header(AUTHORIZATION, bearer(user_id));
    }
    Ok(builder.body(Body::from(serde_json::to_vec(&body)?))?)
}

fn empty_request(method: &str, uri: &str, user_id: Option<Uuid>) -> TestResult<Request<Body>> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(user_id) = user_id {
        builder = builder.header(AUTHORIZATION, bearer(user_id));
    }
    Ok(builder.body(Body::empty())?)
}

async fn assert_error(
    response: Response<Body>,
    status: StatusCode,
    code: &str,
) -> TestResult<Value> {
    assert_eq!(response.status(), status);
    let body = response_json(response).await?;
    assert_eq!(body["error"]["code"], code);
    assert!(body["error"]["request_id"].as_str().is_some());
    assert_eq!(body["error"]["details"], Value::Null);
    Ok(body)
}

async fn response_json(response: Response<Body>) -> TestResult<Value> {
    let bytes = to_bytes(response.into_body(), 64 * 1024).await?;
    if bytes.is_empty() {
        return Err(io::Error::other("response body was empty").into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
