use std::{collections::BTreeSet, io, sync::Arc};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode, header::AUTHORIZATION},
};
use jamye_server::{
    adapters::postgres::{
        notifications::PostgresNotificationsRepository, push::PostgresPushRepository,
        transactions::SqlxTransactionManager,
    },
    application::{
        auth::{AccessIdentity, AccessTokenVerifier, AuthenticationError},
        notifications::{NotificationsDependencies, NotificationsService},
        push::{PushDependencies, PushService},
    },
    transport::http::{
        notifications::{NotificationsHttpState, router as notifications_router},
        push::{PushHttpState, router as push_router},
    },
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    TestResult, postgres_support::TestDatabase, send_authorization::helpers::SendTopology,
};

const INSTALLATION_ID: &str = "task-9-http-device";
const EXPO_TOKEN: &str = "ExponentPushToken[task-9-http]";
const ROTATED_EXPO_TOKEN: &str = "ExponentPushToken[task-9-http-rotated]";

#[tokio::test]
async fn http_boundary_requires_bearer_and_rejects_ambiguous_or_unknown_input() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let router = task_9_router(pool.clone());
    let actor_id = Uuid::new_v4();

    let unauthorized = router
        .clone()
        .oneshot(empty_request("GET", "/api/v1/notifications", None)?)
        .await?;
    assert_error(
        unauthorized,
        StatusCode::UNAUTHORIZED,
        "authentication_required",
    )
    .await?;
    for uri in [
        "/api/v1/notifications?limit=0",
        "/api/v1/notifications?after=not-a-uuid",
        "/api/v1/notifications?limit=10&limit=20",
        "/api/v1/notifications?unknown=1",
    ] {
        let response = router
            .clone()
            .oneshot(empty_request("GET", uri, Some(actor_id))?)
            .await?;
        assert_error(
            response,
            StatusCode::UNPROCESSABLE_ENTITY,
            "request_validation_failed",
        )
        .await?;
    }
    let malformed_notification = router
        .clone()
        .oneshot(empty_request(
            "POST",
            "/api/v1/notifications/not-a-uuid/read",
            Some(actor_id),
        )?)
        .await?;
    assert_error(
        malformed_notification,
        StatusCode::UNPROCESSABLE_ENTITY,
        "request_validation_failed",
    )
    .await?;
    let unauthorized_push = router
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/v1/push/installations",
            None,
            json!({
                "platform": "ios",
                "environment": "development",
                "installation_id": INSTALLATION_ID,
                "expo_token": EXPO_TOKEN,
            }),
        )?)
        .await?;
    assert_error(
        unauthorized_push,
        StatusCode::UNAUTHORIZED,
        "authentication_required",
    )
    .await?;
    let unknown_create_field = router
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/v1/push/installations",
            Some(actor_id),
            json!({
                "platform": "ios",
                "environment": "development",
                "installation_id": INSTALLATION_ID,
                "expo_token": EXPO_TOKEN,
                "provider": "expo",
            }),
        )?)
        .await?;
    assert_error(
        unknown_create_field,
        StatusCode::UNPROCESSABLE_ENTITY,
        "request_validation_failed",
    )
    .await?;
    let unknown_update_field = router
        .oneshot(json_request(
            "PUT",
            &format!("/api/v1/push/installations/{INSTALLATION_ID}"),
            Some(actor_id),
            json!({"expo_token": EXPO_TOKEN, "owner_epoch": 9}),
        )?)
        .await?;
    assert_error(
        unknown_update_field,
        StatusCode::UNPROCESSABLE_ENTITY,
        "request_validation_failed",
    )
    .await?;

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn n1_returns_the_owner_scoped_structured_page_without_private_state() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let topology = SendTopology::new(&pool).await?;
    let response = task_9_router(pool.clone())
        .oneshot(empty_request(
            "GET",
            "/api/v1/notifications?limit=10",
            Some(topology.recipient_id),
        )?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let page = response_json(response).await?;
    assert_exact_keys(&page, &["items", "next_cursor", "unread_count"]);
    assert_eq!(page["unread_count"], 1);
    let notification = &page["items"][0];
    assert_exact_keys(
        notification,
        &[
            "args",
            "conversation_id",
            "created_at",
            "id",
            "read_at",
            "source_cursor",
            "topic_id",
            "type",
        ],
    );
    assert_eq!(notification["id"], topology.notification_id.to_string());
    assert_eq!(notification["type"], "chat_unread");
    assert_eq!(
        notification["args"],
        json!({"sender_display_name": "authorization sender"})
    );
    assert_eq!(
        notification["conversation_id"],
        topology.conversation_id.to_string()
    );
    assert!(notification["source_cursor"].as_str().is_some());
    assert_eq!(notification["read_at"], Value::Null);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn n2_is_idempotent_and_missing_or_foreign_ids_share_one_safe_not_found() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let topology = SendTopology::new(&pool).await?;
    let router = task_9_router(pool.clone());
    let read_uri = format!("/api/v1/notifications/{}/read", topology.notification_id);

    for _ in 0..2 {
        let response = router
            .clone()
            .oneshot(empty_request(
                "POST",
                &read_uri,
                Some(topology.recipient_id),
            )?)
            .await?;
        assert_empty(response, StatusCode::NO_CONTENT).await?;
    }
    for (actor_id, notification_id) in [
        (topology.owner_id, topology.notification_id),
        (topology.recipient_id, Uuid::new_v4()),
    ] {
        let response = router
            .clone()
            .oneshot(empty_request(
                "POST",
                &format!("/api/v1/notifications/{notification_id}/read"),
                Some(actor_id),
            )?)
            .await?;
        assert_error(response, StatusCode::NOT_FOUND, "notification_not_found").await?;
    }
    let read_at = sqlx::query_scalar::<_, Option<time::OffsetDateTime>>(
        "SELECT read_at FROM notifications WHERE id = $1",
    )
    .bind(topology.notification_id)
    .fetch_one(&pool)
    .await?;
    assert!(read_at.is_some());

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn p2_uses_create_then_canonical_upsert_status_and_never_returns_private_fields() -> TestResult
{
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let topology = SendTopology::new(&pool).await?;
    let router = task_9_router(pool.clone());
    let body = json!({
        "platform": "ios",
        "environment": "development",
        "installation_id": INSTALLATION_ID,
        "expo_token": EXPO_TOKEN,
    });

    let created = router
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/v1/push/installations",
            Some(topology.owner_id),
            body.clone(),
        )?)
        .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let installation = response_json(created).await?;
    assert_public_installation(&installation, false);

    let retry = router
        .oneshot(json_request(
            "POST",
            "/api/v1/push/installations",
            Some(topology.owner_id),
            body,
        )?)
        .await?;
    assert_eq!(retry.status(), StatusCode::OK);
    let retry = response_json(retry).await?;
    assert_public_installation(&retry, false);
    assert_eq!(retry["installation_id"], installation["installation_id"]);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn p3_and_p4_are_current_owner_scoped_and_keep_the_public_response_shape() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let topology = SendTopology::new(&pool).await?;
    let router = task_9_router(pool.clone());
    let installation_uri = format!(
        "/api/v1/push/installations/{}",
        topology.public_installation_id
    );

    let updated = router
        .clone()
        .oneshot(json_request(
            "PUT",
            &installation_uri,
            Some(topology.recipient_id),
            json!({
                "expo_token": ROTATED_EXPO_TOKEN,
                "message_preview_enabled": false,
            }),
        )?)
        .await?;
    assert_eq!(updated.status(), StatusCode::OK);
    assert_public_installation(&response_json(updated).await?, false);

    for method in ["PUT", "DELETE"] {
        let response = if method == "PUT" {
            router
                .clone()
                .oneshot(json_request(
                    method,
                    &installation_uri,
                    Some(topology.owner_id),
                    json!({"expo_token": EXPO_TOKEN}),
                )?)
                .await?
        } else {
            router
                .clone()
                .oneshot(empty_request(
                    method,
                    &installation_uri,
                    Some(topology.owner_id),
                )?)
                .await?
        };
        assert_error(
            response,
            StatusCode::NOT_FOUND,
            "push_installation_not_found",
        )
        .await?;
    }
    let deleted = router
        .clone()
        .oneshot(empty_request(
            "DELETE",
            &installation_uri,
            Some(topology.recipient_id),
        )?)
        .await?;
    assert_empty(deleted, StatusCode::NO_CONTENT).await?;
    let retry = router
        .oneshot(empty_request(
            "DELETE",
            &installation_uri,
            Some(topology.recipient_id),
        )?)
        .await?;
    assert_error(retry, StatusCode::NOT_FOUND, "push_installation_not_found").await?;

    pool.close().await;
    database.dispose().await
}

fn task_9_router(pool: PgPool) -> Router {
    let verifier: Arc<dyn AccessTokenVerifier> = Arc::new(TestAccessVerifier);
    let notifications = Arc::new(NotificationsService::new(NotificationsDependencies {
        transactions: Arc::new(SqlxTransactionManager::new(pool.clone())),
        repository: Arc::new(PostgresNotificationsRepository::new(pool.clone())),
    }));
    let push = Arc::new(PushService::new(PushDependencies {
        transactions: Arc::new(SqlxTransactionManager::new(pool.clone())),
        repository: Arc::new(PostgresPushRepository::new(pool)),
    }));
    notifications_router(NotificationsHttpState::new(notifications, verifier.clone()))
        .merge(push_router(PushHttpState::new(push, verifier)))
}

#[derive(Clone, Copy, Debug, Default)]
struct TestAccessVerifier;

impl AccessTokenVerifier for TestAccessVerifier {
    fn verify(&self, token: &str) -> Result<AccessIdentity, AuthenticationError> {
        let user_id = token
            .strip_prefix("task9-")
            .and_then(|value| Uuid::try_parse(value).ok())
            .ok_or(AuthenticationError)?;
        Ok(AccessIdentity::new(user_id, Uuid::nil(), "task-9-test"))
    }
}

fn bearer(user_id: Uuid) -> String {
    format!("Bearer task9-{user_id}")
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
    assert_eq!(body["error"]["details"], Value::Null);
    Ok(())
}

async fn assert_empty(response: Response<Body>, status: StatusCode) -> TestResult {
    assert_eq!(response.status(), status);
    assert!(to_bytes(response.into_body(), 1024).await?.is_empty());
    Ok(())
}

async fn response_json(response: Response<Body>) -> TestResult<Value> {
    let bytes = to_bytes(response.into_body(), 128 * 1024).await?;
    if bytes.is_empty() {
        return Err(io::Error::other("response body was empty").into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}

fn assert_public_installation(value: &Value, preview_enabled: bool) {
    assert_exact_keys(
        value,
        &[
            "disabled_at",
            "environment",
            "installation_id",
            "last_seen_at",
            "message_preview_enabled",
            "platform",
            "provider",
        ],
    );
    assert!(value["installation_id"].as_str().is_some());
    assert_eq!(value["platform"], "ios");
    assert_eq!(value["environment"], "development");
    assert_eq!(value["provider"], "expo");
    assert_eq!(value["message_preview_enabled"], preview_enabled);
    assert!(value["last_seen_at"].as_str().is_some());
    assert_eq!(value["disabled_at"], Value::Null);
}

fn assert_exact_keys(value: &Value, expected: &[&str]) {
    let Some(object) = value.as_object() else {
        panic!("expected a JSON object");
    };
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
}
