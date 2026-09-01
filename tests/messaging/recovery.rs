use std::{
    env, fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{Router, http::StatusCode};
use jamye_server::{
    platform::{
        logging::build_json_subscriber,
        readiness::{DependencyProbe, ProbeFuture, ProbeOutcome, ReadinessService},
    },
    transport::http::composition::router_with_readiness,
};
use serde_json::json;
use sqlx::PgPool;
use tower::ServiceExt;
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;

use crate::{
    messaging_helpers::{
        TestApp, assert_error, counts, events_from, json_body, router_for_unavailable_database,
        send_to, test_identity,
    },
    messaging_http::message_payload,
    postgres_support::TestResult,
};

#[tokio::test]
async fn postgres_unavailability_keeps_liveness_and_returns_safe_503() -> TestResult {
    let persisted = TestApp::new().await?;
    let unavailable = router_for_unavailable_database(test_identity(persisted.fixture.user_id))?;
    let app = health_router(ProbeOutcome::Unreachable).merge(unavailable);
    let chatroom_id = persisted.fixture.chatroom_id;
    let client_msg_id = Uuid::new_v4();

    let live = app
        .clone()
        .oneshot(axum::http::Request::get("/health/live").body(axum::body::Body::empty())?)
        .await?;
    assert_eq!(live.status(), StatusCode::OK);
    let ready = app
        .clone()
        .oneshot(axum::http::Request::get("/health/ready").body(axum::body::Body::empty())?)
        .await?;
    assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);

    let message = send_to(
        &app,
        Some("opaque-test-token"),
        chatroom_id,
        message_payload(client_msg_id, "unavailable"),
        None,
    )
    .await?;
    assert_error(
        message,
        StatusCode::SERVICE_UNAVAILABLE,
        "database_unavailable",
    )
    .await?;
    let delta = events_from(
        &app,
        Some("opaque-test-token"),
        chatroom_id,
        None,
        10,
        Some("1"),
    )
    .await?;
    assert_error(
        delta,
        StatusCode::SERVICE_UNAVAILABLE,
        "database_unavailable",
    )
    .await?;
    assert_eq!(counts(&persisted.pool).await?, (0, 0, 0));

    let retry = persisted
        .send(
            Some(&persisted.fixture.access_token),
            chatroom_id,
            message_payload(client_msg_id, "unavailable"),
            None,
        )
        .await?;
    assert_eq!(retry.status(), StatusCode::CREATED);
    assert_eq!(counts(&persisted.pool).await?, (1, 1, 1));
    persisted.dispose().await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires the task-4a guarded PostgreSQL stop/start card"]
async fn postgres_stop_restart_keeps_the_same_router_alive_and_recovers() -> TestResult {
    let coordination = coordination_directory()?;
    let persisted = TestApp::new().await?;
    let (messaging, recovery_pool) = persisted.restartable_router()?;
    let app = health_router_for_pool(recovery_pool.clone()).merge(messaging);
    let chatroom_id = persisted.fixture.chatroom_id;
    let access_token = persisted.fixture.access_token.clone();
    let client_msg_id = Uuid::new_v4();

    assert_eq!(health_status(&app, "/health/ready").await?, StatusCode::OK);
    signal(&coordination, "ready-to-stop")?;
    wait_for_signal(&coordination, "postgres-stopped").await?;

    assert_eq!(health_status(&app, "/health/live").await?, StatusCode::OK);
    let ready = health_response(&app, "/health/ready").await?;
    assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);
    let ready_body = json_body(ready).await?;
    assert_eq!(ready_body["status"], "not_ready");
    assert_eq!(ready_body["checks"]["postgres"]["status"], "unavailable");
    assert_eq!(ready_body["checks"]["postgres"]["required"], true);
    let message = send_to(
        &app,
        Some(&access_token),
        chatroom_id,
        message_payload(client_msg_id, "stop-restart"),
        None,
    )
    .await?;
    assert_error(
        message,
        StatusCode::SERVICE_UNAVAILABLE,
        "database_unavailable",
    )
    .await?;
    let delta = events_from(&app, Some(&access_token), chatroom_id, None, 10, Some("1")).await?;
    assert_error(
        delta,
        StatusCode::SERVICE_UNAVAILABLE,
        "database_unavailable",
    )
    .await?;

    signal(&coordination, "ready-to-start")?;
    wait_for_signal(&coordination, "postgres-started").await?;
    wait_for_ready(&app).await?;
    assert_eq!(counts(&recovery_pool).await?, (0, 0, 0));

    let retry = send_to(
        &app,
        Some(&access_token),
        chatroom_id,
        message_payload(client_msg_id, "stop-restart"),
        None,
    )
    .await?;
    assert_eq!(retry.status(), StatusCode::CREATED);
    assert_eq!(counts(&recovery_pool).await?, (1, 1, 1));

    persisted
        .dispose_after_postgres_restart(recovery_pool)
        .await
}

#[tokio::test(flavor = "current_thread")]
async fn structured_logs_exclude_message_and_database_secrets() -> TestResult {
    let app = TestApp::new().await?;
    let writer = SharedWriter::default();
    let output = writer.clone();
    let subscriber = build_json_subscriber(writer, "info")?;
    let _guard = tracing::subscriber::set_default(subscriber);
    let _interest_sentinel = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
    let sentinel = "TASK_4A_SENTINEL_MESSAGE_BODY";

    let created = app
        .send(
            Some(&app.fixture.access_token),
            app.fixture.chatroom_id,
            message_payload(Uuid::new_v4(), sentinel),
            None,
        )
        .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let mismatched_idempotency_key = Uuid::new_v4().to_string();
    let mismatch = app
        .send(
            Some(&app.fixture.access_token),
            app.fixture.chatroom_id,
            message_payload(Uuid::new_v4(), sentinel),
            Some(&mismatched_idempotency_key),
        )
        .await?;
    assert_eq!(mismatch.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let unavailable = router_for_unavailable_database(test_identity(Uuid::new_v4()))?;
    let database_error = send_to(
        &unavailable,
        Some("opaque-test-token"),
        Uuid::new_v4(),
        json!({"client_msg_id": Uuid::new_v4(), "body": sentinel}),
        None,
    )
    .await?;
    assert_eq!(database_error.status(), StatusCode::SERVICE_UNAVAILABLE);

    let logs = output.snapshot()?;
    for forbidden in [
        sentinel,
        "INSERT INTO",
        "SELECT ",
        "postgres://",
        "task4a_db_user",
        "task4a_unavailable_db",
        "127.0.0.1:1",
        "connection refused",
        "stack backtrace",
    ] {
        assert!(!logs.contains(forbidden), "logs leaked {forbidden}");
    }
    assert!(logs.contains("request_id"));
    assert!(logs.contains("idempotency_key_mismatch"));
    assert!(logs.contains("database_unavailable"));
    assert_eq!(counts(&app.pool).await?, (1, 1, 1));
    app.dispose().await
}

fn health_router(postgres: ProbeOutcome) -> Router {
    router_with_readiness(ReadinessService::new(
        Arc::new(FixedProbe(postgres)),
        Arc::new(FixedProbe(ProbeOutcome::Unreachable)),
        Arc::new(FixedProbe(ProbeOutcome::Unreachable)),
    ))
}

fn health_router_for_pool(pool: PgPool) -> Router {
    router_with_readiness(ReadinessService::new(
        Arc::new(PoolProbe(pool)),
        Arc::new(FixedProbe(ProbeOutcome::Unreachable)),
        Arc::new(FixedProbe(ProbeOutcome::Unreachable)),
    ))
}

async fn health_status(app: &Router, path: &str) -> TestResult<StatusCode> {
    Ok(health_response(app, path).await?.status())
}

async fn health_response(
    app: &Router,
    path: &str,
) -> TestResult<axum::http::Response<axum::body::Body>> {
    Ok(app
        .clone()
        .oneshot(axum::http::Request::get(path).body(axum::body::Body::empty())?)
        .await?)
}

async fn wait_for_ready(app: &Router) -> TestResult {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if health_status(app, "/health/ready").await? == StatusCode::OK {
                return Ok::<(), Box<dyn std::error::Error + Send + Sync>>(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    match result {
        Ok(result) => result,
        Err(_) => Err(io::Error::other("same router did not recover PostgreSQL readiness").into()),
    }
}

fn coordination_directory() -> TestResult<PathBuf> {
    let path = PathBuf::from(env::var("JAMYE_TASK4A_RECOVERY_COORD_DIR").map_err(|_| {
        io::Error::other("run this ignored test only through the task-4a postgres-recovery card")
    })?);
    if !path.is_dir() {
        return Err(io::Error::other("task-4a recovery coordination directory is absent").into());
    }
    Ok(path)
}

fn signal(directory: &Path, name: &str) -> TestResult {
    fs::write(directory.join(name), b"ready\n")?;
    Ok(())
}

async fn wait_for_signal(directory: &Path, name: &str) -> TestResult {
    let marker = directory.join(name);
    tokio::time::timeout(Duration::from_secs(90), async {
        while !marker.is_file() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| io::Error::other(format!("timed out waiting for {name}")))?;
    Ok(())
}

struct FixedProbe(ProbeOutcome);

impl DependencyProbe for FixedProbe {
    fn check(&self) -> ProbeFuture<'_> {
        Box::pin(async move { self.0 })
    }
}

struct PoolProbe(PgPool);

impl DependencyProbe for PoolProbe {
    fn check(&self) -> ProbeFuture<'_> {
        Box::pin(async move {
            let query = sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&self.0);
            match tokio::time::timeout(Duration::from_millis(750), query).await {
                Ok(Ok(1)) => ProbeOutcome::Reachable,
                _ => ProbeOutcome::Unreachable,
            }
        })
    }
}

#[derive(Clone, Default)]
struct SharedWriter(Arc<Mutex<Vec<u8>>>);

impl SharedWriter {
    fn snapshot(&self) -> io::Result<String> {
        let bytes = self
            .0
            .lock()
            .map_err(|_| io::Error::other("log writer lock poisoned"))?
            .clone();
        String::from_utf8(bytes).map_err(io::Error::other)
    }
}

impl<'a> MakeWriter<'a> for SharedWriter {
    type Writer = SharedWriterGuard;

    fn make_writer(&'a self) -> Self::Writer {
        SharedWriterGuard(self.0.clone())
    }
}

struct SharedWriterGuard(Arc<Mutex<Vec<u8>>>);

impl io::Write for SharedWriterGuard {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("log writer lock poisoned"))?
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
