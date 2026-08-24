use std::{
    error::Error,
    io::{self, Write},
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    routing::get,
};
use jamye_server::{
    config::{AppConfig, ConfigInput},
    platform::{
        logging::{build_json_subscriber, validate_filter},
        readiness::{DependencyProbe, ProbeFuture, ProbeOutcome, ReadinessService},
        request_id::REQUEST_ID_HEADER,
        shutdown::serve_with_graceful_shutdown,
    },
    transport::http::composition::router_with_readiness,
};
use serde_json::Value;
use tokio::{net::TcpListener, sync::Notify};
use tower::ServiceExt;
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;

struct FixedProbe(ProbeOutcome);

impl DependencyProbe for FixedProbe {
    fn check(&self) -> ProbeFuture<'_> {
        Box::pin(async move { self.0 })
    }
}

fn readiness(postgres: ProbeOutcome, redis: ProbeOutcome, minio: ProbeOutcome) -> ReadinessService {
    ReadinessService::new(
        Arc::new(FixedProbe(postgres)),
        Arc::new(FixedProbe(redis)),
        Arc::new(FixedProbe(minio)),
    )
}

#[tokio::test]
async fn liveness_is_unconditional_and_has_server_request_id() -> Result<(), Box<dyn Error>> {
    let app = router_with_readiness(readiness(
        ProbeOutcome::Unreachable,
        ProbeOutcome::Unreachable,
        ProbeOutcome::Unreachable,
    ));
    let request = Request::builder()
        .uri("/health/live")
        .header(REQUEST_ID_HEADER, "caller-controlled")
        .body(Body::empty())?;
    let response = app.oneshot(request).await?;

    assert_eq!(response.status(), StatusCode::OK);
    let request_id = response
        .headers()
        .get(REQUEST_ID_HEADER)
        .ok_or("response did not include a server request ID")?;
    let request_id = request_id.to_str()?;
    let parsed = Uuid::try_parse(request_id)?;
    assert_eq!(parsed.get_version_num(), 4);
    assert_ne!(request_id, "caller-controlled");
    Ok(())
}

#[tokio::test]
async fn readiness_requires_postgres_but_only_degrades_optional_services()
-> Result<(), Box<dyn Error>> {
    let app = router_with_readiness(readiness(
        ProbeOutcome::Reachable,
        ProbeOutcome::Unreachable,
        ProbeOutcome::Unreachable,
    ));
    let response = app
        .oneshot(Request::get("/health/ready").body(Body::empty())?)
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 64 * 1024).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["status"], "ready");
    assert_eq!(json["checks"]["postgres"]["status"], "ready");
    assert_eq!(json["checks"]["redis"]["status"], "degraded");
    assert_eq!(json["checks"]["minio"]["status"], "degraded");
    Ok(())
}

#[tokio::test]
async fn readiness_fails_when_postgres_is_unreachable() -> Result<(), Box<dyn Error>> {
    let app = router_with_readiness(readiness(
        ProbeOutcome::Unreachable,
        ProbeOutcome::Reachable,
        ProbeOutcome::Reachable,
    ));
    let response = app
        .oneshot(Request::get("/health/ready").body(Body::empty())?)
        .await?;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), 64 * 1024).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["status"], "not_ready");
    assert_eq!(json["checks"]["postgres"]["status"], "unavailable");
    Ok(())
}

#[test]
fn config_validates_ranges_and_redacts_connection_urls() -> Result<(), Box<dyn Error>> {
    let input = ConfigInput {
        environment: Some("test".to_owned()),
        database_url: Some("postgres://user:private-password@127.0.0.1/jamye".to_owned()),
        redis_url: Some("redis://:private-password@127.0.0.1/0".to_owned()),
        minio_health_url: Some("http://127.0.0.1:9000/minio/health/live".to_owned()),
        ..ConfigInput::default()
    };
    let config = AppConfig::try_from(input)?;
    let debug = format!("{config:?}");

    assert!(!debug.contains("private-password"));
    assert!(debug.contains("[REDACTED]"));
    assert_eq!(config.listen_address().to_string(), "127.0.0.1:3000");

    let invalid_timeout = ConfigInput {
        environment: Some("test".to_owned()),
        database_url: Some("postgres://127.0.0.1/jamye".to_owned()),
        readiness_timeout_ms: Some("49".to_owned()),
        ..ConfigInput::default()
    };
    let error = AppConfig::try_from(invalid_timeout).err();
    assert_eq!(
        error.as_ref().map(|error| error.key()),
        Some("JAMYE_READINESS_TIMEOUT_MS")
    );
    Ok(())
}

#[test]
fn config_rejects_a_minio_url_that_could_leak_or_redirect() {
    let input = ConfigInput {
        environment: Some("test".to_owned()),
        database_url: Some("postgres://127.0.0.1/jamye".to_owned()),
        minio_health_url: Some("https://user:secret@example.test/elsewhere".to_owned()),
        ..ConfigInput::default()
    };
    let error = AppConfig::try_from(input).err();

    assert_eq!(
        error.as_ref().map(|error| error.key()),
        Some("JAMYE_MINIO_HEALTH_URL")
    );
}

#[test]
fn logging_filter_is_validated_without_echoing_its_value() {
    assert!(validate_filter("jamye_server=info").is_ok());
    assert!(validate_filter("[definitely invalid").is_err());
}

#[test]
fn logging_output_is_structured_json() -> Result<(), Box<dyn Error>> {
    let writer = SharedWriter::default();
    let output = writer.clone();
    let subscriber = build_json_subscriber(writer, "info")?;

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(event_kind = "platform_test", "structured event");
    });

    let line = output.snapshot()?;
    let json: Value = serde_json::from_slice(&line)?;
    assert_eq!(json["event_kind"], "platform_test");
    assert_eq!(json["message"], "structured event");
    Ok(())
}

#[tokio::test]
async fn graceful_shutdown_waits_for_an_in_flight_request() -> Result<(), Box<dyn Error>> {
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let app = slow_router(Arc::clone(&entered), Arc::clone(&release));
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(serve_with_graceful_shutdown(
        listener,
        app,
        async move {
            let _receive_result = shutdown_rx.await;
        },
        Duration::from_secs(2),
    ));

    let request = tokio::spawn(async move {
        reqwest::Client::new()
            .get(format!("http://{address}/slow"))
            .send()
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), entered.notified()).await?;
    if shutdown_tx.send(()).is_err() {
        return Err("server stopped before shutdown was requested".into());
    }
    assert!(!server.is_finished());

    release.notify_one();
    let request_result = tokio::time::timeout(Duration::from_secs(5), request).await?;
    let response = request_result??;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let server_result = tokio::time::timeout(Duration::from_secs(5), server).await?;
    server_result??;
    Ok(())
}

fn slow_router(entered: Arc<Notify>, release: Arc<Notify>) -> Router {
    Router::new().route(
        "/slow",
        get(move || {
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            async move {
                entered.notify_one();
                release.notified().await;
                StatusCode::NO_CONTENT
            }
        }),
    )
}

#[derive(Clone, Default)]
struct SharedWriter {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl SharedWriter {
    fn snapshot(&self) -> io::Result<Vec<u8>> {
        let bytes = self
            .bytes
            .lock()
            .map_err(|_| io::Error::other("log buffer mutex is poisoned"))?;
        Ok(bytes.clone())
    }
}

impl<'writer> MakeWriter<'writer> for SharedWriter {
    type Writer = BufferWriter;

    fn make_writer(&'writer self) -> Self::Writer {
        BufferWriter {
            bytes: Arc::clone(&self.bytes),
        }
    }
}

struct BufferWriter {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl Write for BufferWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.bytes
            .lock()
            .map_err(|_| io::Error::other("log buffer mutex is poisoned"))?
            .write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.bytes
            .lock()
            .map_err(|_| io::Error::other("log buffer mutex is poisoned"))?
            .flush()
    }
}
