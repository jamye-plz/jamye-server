const ID: &str = "00000000-0000-0000-0000-000000000001";
const AUTH_SECRET: &str = "task-12-production-auth-secret-at-least-32-bytes";

#[derive(Clone, Copy)]
enum ExpectedResponse {
    Live,
    Ready,

    RequestValidation,
    AuthenticationRequired,
    WebSocketUpgradeRequired,
}

struct Surface {
    name: &'static str,
    method: Method,
    path: String,
    expected: ExpectedResponse,
}

#[tokio::test]
async fn api_root_requires_validated_auth_config_and_keeps_invalid_or_missing_values_fail_closed()
-> TestResult {
    assert!(
        AuthConfig::try_from(AuthConfigInput::default()).is_err(),
        "missing AuthConfig unexpectedly passed validation"
    );

    let mut invalid = auth_config_input();
    invalid.access_token_secret = Some("too-short".to_owned());
    assert!(
        AuthConfig::try_from(invalid).is_err(),
        "invalid access-token configuration unexpectedly passed validation"
    );

    let config = test_config()?;
    let auth = validated_auth_config()?;
    let response = observe(
        production_router(&config, &auth)?,
        Method::GET,
        "/health/live",
    )
    .await?;
    require(
        response.status == StatusCode::OK && response.body.contains("\"status\":\"live\""),
        "validated AuthConfig did not reach the exact API root used by src/bin/api.rs",
    )
}

#[tokio::test]
async fn api_root_matches_the_complete_frozen_selected_method_path_inventory() -> TestResult {
    let config = test_config()?;
    let auth = validated_auth_config()?;
    let router = production_router(&config, &auth)?;
    let mut failures = Vec::new();

    for surface in selected_surfaces() {
        let response = observe(router.clone(), surface.method, &surface.path).await?;
        if !matches_expected(&response, surface.expected) {
            failures.push(format!(
                "{}: status={} body={}",
                surface.name, response.status, response.body
            ));
        }
    }

    require(
        failures.is_empty(),
        &format!(
            "Task-12 RED: exact API root did not meet the selected handler contracts: {}",
            failures.join(" | ")
        ),
    )
}

#[tokio::test]
async fn api_root_rejects_known_dev_fixture_and_unselected_plugin_canaries() -> TestResult {
    let config = test_config()?;
    let auth = validated_auth_config()?;
    let router = production_router(&config, &auth)?;
    for (name, method, path) in [
        ("dev-fixtures", Method::POST, "/__dev/fixtures/seed"),
        (
            "unselected-plugin-probe",
            Method::GET,
            "/api/v1/plugins/task-12-probe",
        ),
    ] {
        let response = observe(router.clone(), method, path).await?;
        require(
            response.status == StatusCode::NOT_FOUND,
            &format!(
                "Task-12 RED: exact API root unexpectedly resolved {name}: {}",
                response.status
            ),
        )?;
    }
    Ok(())
}

#[tokio::test]
async fn worker_root_constructs_the_fixed_realtime_push_and_cleanup_runner_set() -> TestResult {
    let storage = object_storage_config()?;
    let cleanup = cleanup_config()?;
    let config = worker_config()?;
    let push = push_config()?;
    let result = composition::worker(&config, &push, &storage, &cleanup);
    require(
        result.is_ok(),
        "Task-12 RED: exact worker root is missing the fixed Task-9 push and Task-11 account cleanup runners",
    )
}

struct ObservedResponse {
    status: StatusCode,
    body: String,
}

async fn observe(
    router: axum::Router,
    method: Method,
    path: &str,
) -> Result<ObservedResponse, tower::BoxError> {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())?;
    let response = router.oneshot(request).await?;
    let status = response.status();
    let body = to_bytes(response.into_body(), 64 * 1024).await?;
    Ok(ObservedResponse {
        status,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn matches_expected(response: &ObservedResponse, expected: ExpectedResponse) -> bool {
    match expected {
        ExpectedResponse::Live => {
            response.status == StatusCode::OK && response.body.contains("\"status\":\"live\"")
        }
        ExpectedResponse::Ready => {
            matches!(
                response.status,
                StatusCode::OK | StatusCode::SERVICE_UNAVAILABLE
            ) && response.body.contains("\"status\"")
        }
        ExpectedResponse::RequestValidation => {
            response.status == StatusCode::UNPROCESSABLE_ENTITY
                && response
                    .body
                    .contains("\"code\":\"request_validation_failed\"")
        }
        ExpectedResponse::AuthenticationRequired => {
            response.status == StatusCode::UNAUTHORIZED
                && response
                    .body
                    .contains("\"code\":\"authentication_required\"")
        }
        ExpectedResponse::WebSocketUpgradeRequired => response.status == StatusCode::BAD_REQUEST,
    }
}

fn selected_surfaces() -> Vec<Surface> {
    let protected = ExpectedResponse::AuthenticationRequired;
    [
        (
            "health.live",
            Method::GET,
            "/health/live".to_owned(),
            ExpectedResponse::Live,
        ),
        (
            "health.ready",
            Method::GET,
            "/health/ready".to_owned(),
            ExpectedResponse::Ready,
        ),
        (
            "auth.authorize",
            Method::POST,
            "/api/v1/auth/oauth/kakao/authorize".to_owned(),
            ExpectedResponse::RequestValidation,
        ),
        (
            "auth.exchange",
            Method::POST,
            "/api/v1/auth/oauth/kakao/exchange".to_owned(),
            ExpectedResponse::RequestValidation,
        ),
        (
            "auth.refresh",
            Method::POST,
            "/api/v1/auth/refresh".to_owned(),
            ExpectedResponse::RequestValidation,
        ),
        (
            "auth.logout",
            Method::POST,
            "/api/v1/auth/logout".to_owned(),
            ExpectedResponse::AuthenticationRequired,
        ),
        (
            "profile.get",
            Method::GET,
            "/api/v1/me".to_owned(),
            protected,
        ),
        (
            "profile.patch",
            Method::PATCH,
            "/api/v1/me".to_owned(),
            protected,
        ),
        (
            "account.delete",
            Method::DELETE,
            "/api/v1/me".to_owned(),
            protected,
        ),
        (
            "groups.list",
            Method::GET,
            "/api/v1/groups".to_owned(),
            protected,
        ),
        (
            "groups.create",
            Method::POST,
            "/api/v1/groups".to_owned(),
            protected,
        ),
        (
            "groups.get",
            Method::GET,
            format!("/api/v1/groups/{ID}"),
            protected,
        ),
        (
            "groups.patch",
            Method::PATCH,
            format!("/api/v1/groups/{ID}"),
            protected,
        ),
        (
            "groups.delete",
            Method::DELETE,
            format!("/api/v1/groups/{ID}"),
            protected,
        ),
        (
            "groups.members.list",
            Method::GET,
            format!("/api/v1/groups/{ID}/members"),
            protected,
        ),
        (
            "groups.members.patch",
            Method::PATCH,
            format!("/api/v1/groups/{ID}/members/{ID}"),
            protected,
        ),
        (
            "groups.members.delete",
            Method::DELETE,
            format!("/api/v1/groups/{ID}/members/{ID}"),
            protected,
        ),
        (
            "groups.invites.create",
            Method::POST,
            format!("/api/v1/groups/{ID}/invites"),
            protected,
        ),
        (
            "invites.join",
            Method::POST,
            "/api/v1/invites/task-12-invite-code/join".to_owned(),
            protected,
        ),
        (
            "chatrooms.list",
            Method::GET,
            format!("/api/v1/groups/{ID}/chatrooms"),
            protected,
        ),
        (
            "chatrooms.history",
            Method::GET,
            format!("/api/v1/chatrooms/{ID}/messages"),
            protected,
        ),
        (
            "chatrooms.read",
            Method::POST,
            format!("/api/v1/chatrooms/{ID}/read"),
            protected,
        ),
        (
            "topics.list",
            Method::GET,
            format!("/api/v1/groups/{ID}/topics"),
            protected,
        ),
        (
            "topics.create",
            Method::POST,
            format!("/api/v1/groups/{ID}/topics"),
            protected,
        ),
        (
            "topics.dates",
            Method::GET,
            format!("/api/v1/groups/{ID}/topics/dates"),
            protected,
        ),
        (
            "topics.get",
            Method::GET,
            format!("/api/v1/groups/{ID}/topics/{ID}"),
            protected,
        ),
        (
            "topics.patch",
            Method::PATCH,
            format!("/api/v1/groups/{ID}/topics/{ID}"),
            protected,
        ),
        (
            "topics.tags.list",
            Method::GET,
            format!("/api/v1/groups/{ID}/topics/{ID}/tags"),
            protected,
        ),
        (
            "topics.tags.replace",
            Method::PUT,
            format!("/api/v1/groups/{ID}/topics/{ID}/tags"),
            protected,
        ),
        (
            "topics.media",
            Method::GET,
            format!("/api/v1/topics/{ID}/media"),
            protected,
        ),
        (
            "media.url",
            Method::GET,
            format!("/api/v1/media/{ID}/url"),
            protected,
        ),
        (
            "media.download",
            Method::GET,
            format!("/api/v1/media/{ID}/download"),
            protected,
        ),
        (
            "media.upload.create",
            Method::POST,
            "/api/v1/media/uploads".to_owned(),
            protected,
        ),
        (
            "media.upload.finalize",
            Method::POST,
            format!("/api/v1/media/uploads/{ID}/finalize"),
            protected,
        ),
        (
            "notifications.list",
            Method::GET,
            "/api/v1/notifications".to_owned(),
            protected,
        ),
        (
            "notifications.read",
            Method::POST,
            format!("/api/v1/notifications/{ID}/read"),
            protected,
        ),
        (
            "push.installations.create",
            Method::POST,
            "/api/v1/push/installations".to_owned(),
            protected,
        ),
        (
            "push.installations.update",
            Method::PUT,
            format!("/api/v1/push/installations/{ID}"),
            protected,
        ),
        (
            "push.installations.delete",
            Method::DELETE,
            format!("/api/v1/push/installations/{ID}"),
            protected,
        ),
        (
            "messaging.create",
            Method::POST,
            format!("/api/v1/chatrooms/{ID}/messages"),
            protected,
        ),
        (
            "messaging.delta",
            Method::GET,
            format!("/api/v1/conversations/{ID}/events"),
            protected,
        ),
        (
            "realtime.ticket",
            Method::POST,
            "/api/v1/realtime/tickets".to_owned(),
            protected,
        ),
        (
            "realtime.websocket",
            Method::GET,
            "/api/v1/realtime/ws".to_owned(),
            ExpectedResponse::WebSocketUpgradeRequired,
        ),
    ]
    .into_iter()
    .map(|(name, method, path, expected)| Surface {
        name,
        method,
        path,
        expected,
    })
    .collect()
}

fn test_config() -> TestResult<AppConfig> {
    app_config("postgres://127.0.0.1/jamye_test")
}
