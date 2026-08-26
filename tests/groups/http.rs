use std::{io, sync::Arc, time::Duration};

use axum::{
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode, header::AUTHORIZATION},
};
use jamye_server::{
    application::groups::GroupCreateInput,
    ports::groups::GroupRole,
    transport::http::groups::{GroupsHttpState, router as groups_router},
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    TestResult,
    groups_helpers::{
        DenyRateLimiter, TestAccessVerifier, UnavailableRateLimiter, bearer, create_group, harness,
        harness_with_limiter, insert_member, insert_user,
    },
    postgres_support::TestDatabase,
};

#[tokio::test]
async fn group_http_crud_paginates_and_enforces_membership_without_disclosure() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "HTTP 소유자").await?;
    let outsider_id = insert_user(&pool, "HTTP 외부인").await?;
    let router = groups_router(GroupsHttpState::new(
        fixture.service.clone(),
        Arc::new(TestAccessVerifier),
    ));

    let unauthorized = router
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/v1/groups",
            None,
            json!({"name": "인증 없는 그룹"}),
        )?)
        .await?;
    assert_error(
        unauthorized,
        StatusCode::UNAUTHORIZED,
        "authentication_required",
    )
    .await?;

    let created = router
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/v1/groups",
            Some(owner_id),
            json!({"name": "첫 그룹"}),
        )?)
        .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = response_json(created).await?;
    let first_id = uuid_field(&created, "id")?;
    assert_eq!(created["name"], "첫 그룹");
    assert_eq!(created["owner_id"], owner_id.to_string());
    assert_eq!(created["max_members"], 12);
    assert_eq!(created["member_count"], 1);
    assert!(created["main_chatroom_id"].as_str().is_some());

    let second = fixture
        .service
        .create_group(
            owner_id,
            GroupCreateInput {
                name: "둘째 그룹".to_owned(),
            },
        )
        .await?;
    let first_page = router
        .clone()
        .oneshot(empty_request(
            "GET",
            "/api/v1/groups?limit=1",
            Some(owner_id),
        )?)
        .await?;
    assert_eq!(first_page.status(), StatusCode::OK);
    let first_page = response_json(first_page).await?;
    assert_eq!(first_page["items"].as_array().map(Vec::len), Some(1));
    let cursor = first_page["next_cursor"]
        .as_str()
        .ok_or_else(|| io::Error::other("first group page did not return a cursor"))?;
    let first_page_id = first_page["items"][0]["id"]
        .as_str()
        .ok_or_else(|| io::Error::other("first group page did not return an id"))?;
    let second_page = router
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!("/api/v1/groups?limit=1&after={cursor}"),
            Some(owner_id),
        )?)
        .await?;
    assert_eq!(second_page.status(), StatusCode::OK);
    let second_page = response_json(second_page).await?;
    let second_page_id = second_page["items"][0]["id"]
        .as_str()
        .ok_or_else(|| io::Error::other("second group page did not return an id"))?;
    assert_ne!(first_page_id, second_page_id);
    assert!([first_id.to_string(), second.id.to_string()].contains(&first_page_id.to_owned()));
    assert!([first_id.to_string(), second.id.to_string()].contains(&second_page_id.to_owned()));

    let denied = router
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!("/api/v1/groups/{first_id}"),
            Some(outsider_id),
        )?)
        .await?;
    assert_error(denied, StatusCode::FORBIDDEN, "membership_required").await?;

    let members = router
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!("/api/v1/groups/{first_id}/members"),
            Some(owner_id),
        )?)
        .await?;
    assert_eq!(members.status(), StatusCode::OK);
    let members = response_json(members).await?;
    assert_eq!(members["items"][0]["user_id"], owner_id.to_string());
    assert_eq!(members["items"][0]["role"], "owner");
    assert!(members["items"][0].get("membership_id").is_none());

    let renamed = router
        .oneshot(json_request(
            "PATCH",
            &format!("/api/v1/groups/{first_id}"),
            Some(owner_id),
            json!({"name": "바뀐 그룹"}),
        )?)
        .await?;
    assert_eq!(renamed.status(), StatusCode::OK);
    assert_eq!(response_json(renamed).await?["name"], "바뀐 그룹");

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn role_invite_join_removal_and_delete_routes_preserve_authority() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "역할 소유자").await?;
    let successor_id = insert_user(&pool, "역할 후임자").await?;
    let joiner_id = insert_user(&pool, "HTTP 가입자").await?;
    let group = create_group(&fixture, owner_id).await?;
    insert_member(&pool, group.id, successor_id, GroupRole::Member).await?;
    let router = groups_router(GroupsHttpState::new(
        fixture.service.clone(),
        Arc::new(TestAccessVerifier),
    ));

    let transfer = router
        .clone()
        .oneshot(json_request(
            "PATCH",
            &format!("/api/v1/groups/{}/members/{successor_id}", group.id),
            Some(owner_id),
            json!({"role": "owner"}),
        )?)
        .await?;
    assert_eq!(transfer.status(), StatusCode::NO_CONTENT);

    let former_owner_delete = router
        .clone()
        .oneshot(empty_request(
            "DELETE",
            &format!("/api/v1/groups/{}", group.id),
            Some(owner_id),
        )?)
        .await?;
    assert_error(former_owner_delete, StatusCode::FORBIDDEN, "owner_required").await?;

    let issued = router
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/api/v1/groups/{}/invites", group.id),
            Some(successor_id),
            json!({"max_uses": 1}),
        )?)
        .await?;
    assert_eq!(issued.status(), StatusCode::CREATED);
    let issued = response_json(issued).await?;
    let code = issued["code"]
        .as_str()
        .ok_or_else(|| io::Error::other("invite response omitted code"))?;

    let joined = router
        .clone()
        .oneshot(empty_request(
            "POST",
            &format!("/api/v1/invites/{code}/join"),
            Some(joiner_id),
        )?)
        .await?;
    assert_eq!(joined.status(), StatusCode::OK);
    let joined = response_json(joined).await?;
    assert_eq!(joined["joined"], true);
    assert!(joined["membership_id"].as_str().is_some());

    let removed = router
        .clone()
        .oneshot(empty_request(
            "DELETE",
            &format!("/api/v1/groups/{}/members/{joiner_id}", group.id),
            Some(successor_id),
        )?)
        .await?;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    let former_owner_left = router
        .clone()
        .oneshot(empty_request(
            "DELETE",
            &format!("/api/v1/groups/{}/members/{owner_id}", group.id),
            Some(owner_id),
        )?)
        .await?;
    assert_eq!(former_owner_left.status(), StatusCode::NO_CONTENT);

    let deleted = router
        .clone()
        .oneshot(empty_request(
            "DELETE",
            &format!("/api/v1/groups/{}", group.id),
            Some(successor_id),
        )?)
        .await?;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    let hidden = router
        .oneshot(empty_request(
            "GET",
            &format!("/api/v1/groups/{}", group.id),
            Some(successor_id),
        )?)
        .await?;
    assert_error(hidden, StatusCode::NOT_FOUND, "group_not_found").await?;

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn invite_http_returns_stable_429_and_fail_closed_503_before_mutation() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let allowed = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "HTTP 제한 소유자").await?;
    let group = create_group(&allowed, owner_id).await?;

    let denied = harness_with_limiter(
        pool.clone(),
        Arc::new(DenyRateLimiter(Duration::from_millis(2_001))),
    )?;
    let denied_router = groups_router(GroupsHttpState::new(
        denied.service,
        Arc::new(TestAccessVerifier),
    ));
    let limited = denied_router
        .oneshot(json_request(
            "POST",
            &format!("/api/v1/groups/{}/invites", group.id),
            Some(owner_id),
            json!({}),
        )?)
        .await?;
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        limited
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok()),
        Some("3")
    );
    assert_error(
        limited,
        StatusCode::TOO_MANY_REQUESTS,
        "rate_limit_exceeded",
    )
    .await?;

    let unavailable = harness_with_limiter(pool.clone(), Arc::new(UnavailableRateLimiter))?;
    let unavailable_router = groups_router(GroupsHttpState::new(
        unavailable.service,
        Arc::new(TestAccessVerifier),
    ));
    let unavailable = unavailable_router
        .oneshot(json_request(
            "POST",
            &format!("/api/v1/groups/{}/invites", group.id),
            Some(owner_id),
            json!({}),
        )?)
        .await?;
    assert_error(
        unavailable,
        StatusCode::SERVICE_UNAVAILABLE,
        "rate_limit_unavailable",
    )
    .await?;
    let invites: i64 = sqlx::query_scalar("SELECT count(*) FROM invites")
        .fetch_one(&pool)
        .await?;
    assert_eq!(invites, 0);

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

async fn assert_error(response: Response<Body>, status: StatusCode, code: &str) -> TestResult {
    assert_eq!(response.status(), status);
    let body = response_json(response).await?;
    assert_eq!(body["error"]["code"], code);
    assert!(body["error"]["request_id"].as_str().is_some());
    Ok(())
}

async fn response_json(response: Response<Body>) -> TestResult<Value> {
    Ok(serde_json::from_slice(
        &to_bytes(response.into_body(), 32 * 1024).await?,
    )?)
}

fn uuid_field(value: &Value, field: &str) -> TestResult<Uuid> {
    Ok(Uuid::try_parse(value[field].as_str().ok_or_else(
        || io::Error::other(format!("response omitted UUID field {field}")),
    )?)?)
}
