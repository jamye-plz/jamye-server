use std::{io, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode, header::AUTHORIZATION},
};
use jamye_server::transport::http::topics::{TopicsHttpState, router as topics_router};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    TestResult,
    postgres_support::TestDatabase,
    topic_helpers::{TestAccessVerifier, bearer, harness, topology},
};

#[tokio::test]
async fn t1_through_t7_http_use_the_locked_authenticated_mobile_shapes() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let harness = harness(pool.clone());
    let router = topics_router(TopicsHttpState::new(
        harness.service,
        Arc::new(TestAccessVerifier),
    ));
    let key = Uuid::new_v4();
    let create_uri = format!("/api/v1/groups/{}/topics", fixture.group_id);

    let unauthorized = router
        .clone()
        .oneshot(json_request(
            "POST",
            &create_uri,
            None,
            Some(key),
            json!({"title": "인증 없는 주제"}),
        )?)
        .await?;
    assert_error(
        unauthorized,
        StatusCode::UNAUTHORIZED,
        "authentication_required",
    )
    .await?;
    let missing_key = router
        .clone()
        .oneshot(json_request(
            "POST",
            &create_uri,
            Some(fixture.author_id),
            None,
            json!({"title": "키 없는 주제"}),
        )?)
        .await?;
    assert_error(
        missing_key,
        StatusCode::UNPROCESSABLE_ENTITY,
        "request_validation_failed",
    )
    .await?;

    let created = router
        .clone()
        .oneshot(json_request(
            "POST",
            &create_uri,
            Some(fixture.author_id),
            Some(key),
            json!({"title": "  HTTP 주제  "}),
        )?)
        .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = response_json(created).await?;
    let topic_id = uuid_field(&created, "id")?;
    assert_eq!(created["title"], "HTTP 주제");
    assert_eq!(created["status"], "seed");
    assert_eq!(created["body"], Value::Null);
    assert_eq!(created["tags"], json!([]));
    assert!(created.get("media").is_none());
    assert_eq!(created["unread"], false);

    let retry = router
        .clone()
        .oneshot(json_request(
            "POST",
            &create_uri,
            Some(fixture.author_id),
            Some(key),
            json!({"title": "HTTP 주제"}),
        )?)
        .await?;
    assert_eq!(retry.status(), StatusCode::OK);
    assert_eq!(response_json(retry).await?["id"], topic_id.to_string());
    let conflict = router
        .clone()
        .oneshot(json_request(
            "POST",
            &create_uri,
            Some(fixture.author_id),
            Some(key),
            json!({"title": "다른 HTTP 주제"}),
        )?)
        .await?;
    assert_error(conflict, StatusCode::CONFLICT, "topic_idempotency_conflict").await?;

    let list = router
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!("{create_uri}?limit=10"),
            Some(fixture.member_id),
        )?)
        .await?;
    assert_eq!(list.status(), StatusCode::OK);
    let list = response_json(list).await?;
    assert_eq!(list["items"][0]["id"], topic_id.to_string());
    assert_eq!(list["items"][0]["unread"], true);
    let dates = router
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!("{create_uri}/dates?limit=10"),
            Some(fixture.member_id),
        )?)
        .await?;
    assert_eq!(dates.status(), StatusCode::OK);
    let dates = response_json(dates).await?;
    assert!(dates["today"].as_str().is_some());
    assert!(
        dates["dates"]
            .as_array()
            .is_some_and(|dates| !dates.is_empty())
    );

    let topic_uri = format!("{create_uri}/{topic_id}");
    let detail = router
        .clone()
        .oneshot(empty_request("GET", &topic_uri, Some(fixture.member_id))?)
        .await?;
    assert_eq!(detail.status(), StatusCode::OK);
    assert_eq!(
        response_json(detail).await?["author_id"],
        fixture.author_id.to_string()
    );
    for actor_id in [fixture.owner_id, fixture.member_id] {
        let denied = router
            .clone()
            .oneshot(json_request(
                "PATCH",
                &topic_uri,
                Some(actor_id),
                None,
                json!({"title": "권한 없음"}),
            )?)
            .await?;
        assert_error(denied, StatusCode::FORBIDDEN, "topic_author_required").await?;
    }
    let patched = router
        .clone()
        .oneshot(json_request(
            "PATCH",
            &topic_uri,
            Some(fixture.author_id),
            None,
            json!({"title": "  수정 제목  ", "body": "수정 본문"}),
        )?)
        .await?;
    assert_eq!(patched.status(), StatusCode::OK);
    let patched = response_json(patched).await?;
    assert_eq!(patched["title"], "수정 제목");
    assert_eq!(patched["status"], "enriched");

    let tags_uri = format!("{topic_uri}/tags");
    for actor_id in [fixture.owner_id, fixture.member_id] {
        let denied = router
            .clone()
            .oneshot(json_request(
                "PUT",
                &tags_uri,
                Some(actor_id),
                None,
                json!({"tags": [{"tag": "친구", "source": "user", "confidence": null}]}),
            )?)
            .await?;
        assert_error(denied, StatusCode::FORBIDDEN, "topic_author_required").await?;
    }
    let tags = router
        .clone()
        .oneshot(json_request(
            "PUT",
            &tags_uri,
            Some(fixture.author_id),
            None,
            json!({"tags": [{"tag": "친구", "source": "user", "confidence": null}]}),
        )?)
        .await?;
    assert_eq!(tags.status(), StatusCode::OK);
    assert_eq!(response_json(tags).await?["items"][0]["tag"], "친구");
    let tags = router
        .clone()
        .oneshot(empty_request("GET", &tags_uri, Some(fixture.member_id))?)
        .await?;
    assert_eq!(tags.status(), StatusCode::OK);
    assert_eq!(response_json(tags).await?["items"][0]["source"], "user");

    let outsider = router
        .oneshot(empty_request("GET", &topic_uri, Some(fixture.outsider_id))?)
        .await?;
    assert_error(outsider, StatusCode::FORBIDDEN, "membership_required").await?;

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn t1_through_t8_http_use_the_locked_authenticated_mobile_shapes() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let harness = harness(pool.clone());
    let router = topics_router(TopicsHttpState::new(
        harness.service,
        Arc::new(TestAccessVerifier),
    ));
    let create_uri = format!("/api/v1/groups/{}/topics", fixture.group_id);
    let create_key = Uuid::new_v4();
    let created = router
        .clone()
        .oneshot(json_request(
            "POST",
            &create_uri,
            Some(fixture.author_id),
            Some(create_key),
            json!({"title": "삭제 주제"}),
        )?)
        .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = response_json(created).await?;
    let topic_id = uuid_field(&created, "id")?;
    let topic_chatroom_id = uuid_field(&created, "chatroom_id")?;
    let topic_uri = format!("{create_uri}/{topic_id}");
    let tags_uri = format!("{topic_uri}/tags");
    let tags = router
        .clone()
        .oneshot(json_request(
            "PUT",
            &tags_uri,
            Some(fixture.author_id),
            None,
            json!({"tags": [{"tag": "삭제확인", "source": "user", "confidence": null}]}),
        )?)
        .await?;
    assert_eq!(tags.status(), StatusCode::OK);
    sqlx::query(
        "INSERT INTO chatroom_reads (id, user_id, chatroom_id, last_read_cursor) \
         VALUES ($1, $2, $3, 1)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.member_id)
    .bind(topic_chatroom_id)
    .execute(&pool)
    .await?;

    for actor_id in [fixture.owner_id, fixture.member_id] {
        let denied = router
            .clone()
            .oneshot(empty_request("DELETE", &topic_uri, Some(actor_id))?)
            .await?;
        assert_error(denied, StatusCode::FORBIDDEN, "topic_author_required").await?;
    }

    let deleted = router
        .clone()
        .oneshot(empty_request(
            "DELETE",
            &topic_uri,
            Some(fixture.author_id),
        )?)
        .await?;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    let retry = router
        .clone()
        .oneshot(empty_request(
            "DELETE",
            &topic_uri,
            Some(fixture.author_id),
        )?)
        .await?;
    assert_eq!(retry.status(), StatusCode::NO_CONTENT);
    let hidden_from_nonauthor = router
        .clone()
        .oneshot(empty_request(
            "DELETE",
            &topic_uri,
            Some(fixture.member_id),
        )?)
        .await?;
    assert_error(
        hidden_from_nonauthor,
        StatusCode::NOT_FOUND,
        "topic_not_found",
    )
    .await?;
    let idempotent_resend = router
        .clone()
        .oneshot(json_request(
            "POST",
            &create_uri,
            Some(fixture.author_id),
            Some(create_key),
            json!({"title": "삭제 주제"}),
        )?)
        .await?;
    assert_error(
        idempotent_resend,
        StatusCode::CONFLICT,
        "topic_idempotency_conflict",
    )
    .await?;

    let detail = router
        .clone()
        .oneshot(empty_request("GET", &topic_uri, Some(fixture.member_id))?)
        .await?;
    assert_error(detail, StatusCode::NOT_FOUND, "topic_not_found").await?;
    let tags_after_delete = router
        .clone()
        .oneshot(empty_request("GET", &tags_uri, Some(fixture.member_id))?)
        .await?;
    assert_error(tags_after_delete, StatusCode::NOT_FOUND, "topic_not_found").await?;
    let list = router
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!("{create_uri}?limit=10"),
            Some(fixture.member_id),
        )?)
        .await?;
    assert_eq!(list.status(), StatusCode::OK);
    let list = response_json(list).await?;
    assert!(
        !list["items"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["id"] == topic_id.to_string()))
    );

    let (title, body_is_null, topic_deleted) = sqlx::query_as::<_, (String, bool, bool)>(
        "SELECT title, body IS NULL, deleted_at IS NOT NULL FROM topics WHERE id = $1",
    )
    .bind(topic_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(title, "");
    assert!(body_is_null);
    assert!(topic_deleted);
    let chatroom_deleted: bool =
        sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM chatrooms WHERE id = $1")
            .bind(topic_chatroom_id)
            .fetch_one(&pool)
            .await?;
    assert!(chatroom_deleted);
    let tag_state: (i64, i64, bool) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE deleted_at IS NULL), \
                bool_and(tag LIKE 'deleted-%') \
         FROM topic_tags WHERE topic_id = $1",
    )
    .bind(topic_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(tag_state, (1, 0, true));
    let read_counts: (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE deleted_at IS NULL) \
         FROM chatroom_reads WHERE chatroom_id = $1",
    )
    .bind(topic_chatroom_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(read_counts, (2, 0));

    let topic_deleted_in_topic_room: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM conversation_events \
         WHERE conversation_id = $1 AND event_type = 'topic.deleted'",
    )
    .bind(topic_chatroom_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(topic_deleted_in_topic_room, 0);
    let (topic_event_id, topic_payload) = sqlx::query_as::<_, (Uuid, Value)>(
        "SELECT id, payload FROM conversation_events \
         WHERE conversation_id = $1 AND event_type = 'topic.deleted'",
    )
    .bind(fixture.main_chatroom_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(topic_payload["topic_id"], topic_id.to_string());
    assert_eq!(
        topic_payload["topic_chatroom_id"],
        topic_chatroom_id.to_string()
    );
    assert_eq!(topic_payload["deleted_by"], fixture.author_id.to_string());
    let (topic_outbox_aggregate_id, topic_outbox_payload) = sqlx::query_as::<_, (Uuid, Value)>(
        "SELECT aggregate_id, payload FROM outbox_events \
         WHERE event_type = 'topic.deleted' AND conversation_event_id = $1",
    )
    .bind(topic_event_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(topic_outbox_aggregate_id, fixture.main_chatroom_id);
    assert_eq!(topic_outbox_payload["type"], "topic.deleted");
    assert_eq!(
        topic_outbox_payload["conversation_id"],
        fixture.main_chatroom_id.to_string()
    );
    assert_eq!(
        topic_outbox_payload["data"]["topic_id"],
        topic_id.to_string()
    );
    let notification_side_effects: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
             (SELECT COUNT(*) FROM notifications \
              WHERE conversation_id = $1 AND type = 'chat_unread' AND deleted_at IS NULL), \
             (SELECT COUNT(*) FROM notifications \
              WHERE topic_id = $2 AND type = 'new_topic' AND deleted_at IS NULL), \
             (SELECT COUNT(*) FROM push_delivery_intents WHERE source_event_id = $3)",
    )
    .bind(fixture.main_chatroom_id)
    .bind(topic_id)
    .bind(topic_event_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(notification_side_effects, (0, 0, 0));
    let announcement_deleted_events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM conversation_events \
         WHERE conversation_id = $1 \
           AND event_type = 'message.deleted' \
           AND payload ->> 'reason' = 'topic_deleted'",
    )
    .bind(fixture.main_chatroom_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(announcement_deleted_events, 1);

    pool.close().await;
    database.dispose().await
}

fn json_request(
    method: &str,
    uri: &str,
    user_id: Option<Uuid>,
    idempotency_key: Option<Uuid>,
    body: Value,
) -> TestResult<Request<Body>> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(user_id) = user_id {
        builder = builder.header(AUTHORIZATION, bearer(user_id));
    }
    if let Some(key) = idempotency_key {
        builder = builder.header("Idempotency-Key", key.to_string());
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

async fn assert_error(response: Response<Body>, status: StatusCode, code: &str) -> TestResult {
    assert_eq!(response.status(), status);
    let body = response_json(response).await?;
    assert_eq!(body["error"]["code"], code);
    assert!(body["error"]["request_id"].as_str().is_some());
    assert_eq!(body["error"]["details"], Value::Null);
    Ok(())
}

async fn response_json(response: Response<Body>) -> TestResult<Value> {
    let bytes = to_bytes(response.into_body(), 128 * 1024).await?;
    if bytes.is_empty() {
        return Err(io::Error::other("response body was empty").into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}

fn uuid_field(value: &Value, field: &str) -> TestResult<Uuid> {
    Ok(Uuid::try_parse(value[field].as_str().ok_or_else(
        || io::Error::other(format!("response omitted UUID field {field}")),
    )?)?)
}
