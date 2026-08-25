use std::{
    env, fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use axum::Router;
use futures_util::{SinkExt, StreamExt};
use jamye_server::{
    adapters::{
        postgres::{
            messaging::PostgresMessagingRepository, realtime::PostgresRealtimeRepository,
            transactions::SqlxTransactionManager,
        },
        redis::realtime::{OsTicketCredentialSource, RedisRealtimeAdapter},
    },
    application::{
        auth::{AccessIdentity, AccessTokenVerifier, AuthenticationError},
        messaging::MessagingService,
        realtime::{OutboxWorker, OutboxWorkerConfig, RealtimeTicketService, SystemClock},
    },
    domain::messaging::{CanonicalMessage, EventPage, MessageCreatedEvent},
    transport::{
        http::{
            auth::AuthVerifierState,
            messaging::{MessagingHttpState, router as messaging_router},
            realtime::{RealtimeHttpState, router as realtime_router},
        },
        realtime::{CLOSE_REALTIME_AUTH_FAILED, LocalRealtimeHub},
    },
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use tokio::{net::TcpListener, sync::oneshot};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Error as WebSocketError, Message as ClientMessage},
};
use url::Url;
use uuid::Uuid;

use crate::{TestResult, fixture_support::insert_owner_fixture, postgres_support::TestDatabase};

const TEST_BEARER: &str = "task-4b-recovery-bearer";

#[tokio::test]
#[ignore = "the task-4b Redis recovery card coordinates the guarded container lifecycle"]
async fn redis_stop_restart_keeps_postgres_correctness_and_same_router_recovery() -> TestResult {
    let coordination_dir = recovery_coordination_dir()?;
    let redis_url = guarded_redis_url()?;
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = insert_owner_fixture(&pool).await?;
    let identity = AccessIdentity::new(fixture.user_id, Uuid::new_v4(), "task-4b-recovery")
        .with_access_token_expiry(OffsetDateTime::now_utc() + time::Duration::minutes(5));
    let auth = AuthVerifierState::new(Arc::new(StaticVerifier(identity)));
    let messaging = Arc::new(MessagingService::new(
        Arc::new(SqlxTransactionManager::new(pool.clone())),
        Arc::new(PostgresMessagingRepository::new(pool.clone())),
    ));
    let postgres_realtime = Arc::new(PostgresRealtimeRepository::new(pool.clone()));
    let redis_realtime = Arc::new(RedisRealtimeAdapter::new(&redis_url)?);
    let tickets = Arc::new(RealtimeTicketService::new(
        redis_realtime.clone(),
        Arc::new(OsTicketCredentialSource),
        Arc::new(SystemClock),
    ));
    let hub = LocalRealtimeHub::default();
    let application = Router::new()
        .merge(messaging_router(MessagingHttpState::new(
            messaging,
            auth.clone(),
        )))
        .merge(realtime_router(RealtimeHttpState::new(
            tickets,
            hub.clone(),
            postgres_realtime.clone(),
            auth,
        )));
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, application)
            .with_graceful_shutdown(async move {
                let _ = shutdown_receiver.await;
            })
            .await
    });
    let base_url = format!("http://{address}");
    let client = reqwest::Client::new();
    let lost_ticket = issue_ticket(&client, &base_url).await?;
    let client_msg_id = Uuid::new_v4();
    let canonical = post_message(
        &client,
        &base_url,
        fixture.chatroom_id,
        client_msg_id,
        reqwest::StatusCode::CREATED,
    )
    .await?;
    let worker = OutboxWorker::new(
        postgres_realtime,
        redis_realtime.clone(),
        OutboxWorkerConfig {
            claim_owner: "task-4b-recovery".to_owned(),
            batch_size: 10,
            lease_duration: Duration::from_secs(2),
            publish_timeout: Duration::from_millis(200),
            lease_safety_margin: Duration::from_millis(100),
            retry_delay: Duration::from_millis(100),
            poll_interval: Duration::from_millis(10),
            max_attempts: 3,
        },
    )?;

    write_marker(&coordination_dir, "ready-to-stop")?;
    wait_for_marker(&coordination_dir, "redis-stopped").await?;

    let failed_publish = worker.run_once().await?;
    assert_eq!(failed_publish.claimed, 1);
    assert_eq!(failed_publish.published, 0);
    assert_eq!(failed_publish.retries, 1);
    assert_outbox_state(&pool, "pending", 1, Some("redis_unavailable")).await?;

    let unavailable_ticket = client
        .post(format!("{base_url}/api/v1/realtime/tickets"))
        .bearer_auth(TEST_BEARER)
        .header("x-jamye-contract-version", "1")
        .send()
        .await?;
    assert_safe_realtime_unavailable(unavailable_ticket).await?;
    match connect_async(format!(
        "ws://{address}/api/v1/realtime/ws?ticket={}",
        lost_ticket.ticket
    ))
    .await
    {
        Err(WebSocketError::Http(response)) => {
            assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
        }
        Ok(_) => {
            return Err(io::Error::other(
                "WebSocket handshake unexpectedly succeeded during Redis outage",
            )
            .into());
        }
        Err(_) => {
            return Err(io::Error::other(
                "WebSocket handshake failed without the required safe 503 response",
            )
            .into());
        }
    }

    let retry = post_message(
        &client,
        &base_url,
        fixture.chatroom_id,
        client_msg_id,
        reqwest::StatusCode::OK,
    )
    .await?;
    assert_eq!(retry, canonical);
    let delta = delta_page(&client, &base_url, fixture.chatroom_id).await?;
    assert_eq!(delta.items.len(), 1);

    write_marker(&coordination_dir, "ready-to-start")?;
    wait_for_marker(&coordination_dir, "redis-started").await?;

    let (mut lost_socket, _) = connect_async(format!(
        "ws://{address}/api/v1/realtime/ws?ticket={}",
        lost_ticket.ticket
    ))
    .await?;
    let lost_close = next_message(&mut lost_socket).await?;
    match lost_close {
        ClientMessage::Close(Some(frame)) => {
            assert_eq!(u16::from(frame.code), CLOSE_REALTIME_AUTH_FAILED);
            assert_eq!(frame.reason.as_str(), "realtime_auth_failed");
        }
        other => {
            return Err(io::Error::other(format!(
                "expected restart-lost ticket to close 4401, got {other:?}"
            ))
            .into());
        }
    }

    let mut subscriber = redis_realtime.event_subscriber().await?;
    let forward_hub = hub.clone();
    let forwarder = tokio::spawn(async move {
        let event = subscriber
            .next_event()
            .await
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::other("Redis subscriber ended during recovery"))?;
        let conversation_id = event.conversation_id;
        let payload = serde_json::to_string(&event).map_err(io::Error::other)?;
        forward_hub.publish(conversation_id, payload).await;
        Ok::<(), io::Error>(())
    });
    let recovered_ticket = issue_ticket(&client, &base_url).await?;
    let (mut recovered_socket, _) = connect_async(format!(
        "ws://{address}/api/v1/realtime/ws?ticket={}",
        recovered_ticket.ticket
    ))
    .await?;
    send_subscribe(&mut recovered_socket, fixture.chatroom_id).await?;
    let subscribed: Value = serde_json::from_str(&next_text(&mut recovered_socket).await?)?;
    assert_eq!(subscribed["type"], "subscribed");

    let recovered_publish = worker.run_once().await?;
    assert_eq!(recovered_publish.claimed, 1);
    assert_eq!(recovered_publish.published, 1);
    forwarder.await??;
    let realtime_event: MessageCreatedEvent =
        serde_json::from_str(&next_text(&mut recovered_socket).await?)?;
    assert_eq!(realtime_event.data, canonical);
    assert_outbox_state(&pool, "published", 1, None).await?;

    recovered_socket.close(None).await?;
    let _ = shutdown_sender.send(());
    server.await??;
    pool.close().await;
    database.dispose().await
}

async fn issue_ticket(client: &reqwest::Client, base_url: &str) -> TestResult<TicketResponse> {
    let response = client
        .post(format!("{base_url}/api/v1/realtime/tickets"))
        .bearer_auth(TEST_BEARER)
        .header("x-jamye-contract-version", "1")
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let ticket: TicketResponse = serde_json::from_slice(&response.bytes().await?)?;
    assert_eq!(ticket.contract_version, "1");
    assert!(!ticket.expires_at.is_empty());
    Ok(ticket)
}

async fn post_message(
    client: &reqwest::Client,
    base_url: &str,
    chatroom_id: Uuid,
    client_msg_id: Uuid,
    expected_status: reqwest::StatusCode,
) -> TestResult<CanonicalMessage> {
    let response = client
        .post(format!(
            "{base_url}/api/v1/chatrooms/{chatroom_id}/messages"
        ))
        .bearer_auth(TEST_BEARER)
        .header("content-type", "application/json")
        .header("idempotency-key", client_msg_id.to_string())
        .body(
            json!({
                "client_msg_id": client_msg_id,
                "body": "task-4b Redis recovery",
                "media": [],
            })
            .to_string(),
        )
        .send()
        .await?;
    assert_eq!(response.status(), expected_status);
    Ok(serde_json::from_slice(&response.bytes().await?)?)
}

async fn delta_page(
    client: &reqwest::Client,
    base_url: &str,
    conversation_id: Uuid,
) -> TestResult<EventPage> {
    let response = client
        .get(format!(
            "{base_url}/api/v1/conversations/{conversation_id}/events?limit=10"
        ))
        .bearer_auth(TEST_BEARER)
        .header("x-jamye-contract-version", "1")
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    Ok(serde_json::from_slice(&response.bytes().await?)?)
}

async fn assert_safe_realtime_unavailable(response: reqwest::Response) -> TestResult {
    assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = serde_json::from_slice(&response.bytes().await?)?;
    assert_eq!(body["error"]["code"], "realtime_unavailable");
    assert!(body["error"]["details"].is_null());
    assert_eq!(body.as_object().map(|object| object.len()), Some(1));
    Ok(())
}

async fn assert_outbox_state(
    pool: &PgPool,
    status: &str,
    attempt_count: i32,
    error_code: Option<&str>,
) -> TestResult {
    let row = sqlx::query_as::<_, (String, i32, Option<String>)>(
        "SELECT status, attempt_count, last_error_code \
         FROM outbox_events \
         ORDER BY created_at, id \
         LIMIT 1",
    )
    .fetch_one(pool)
    .await?;
    assert_eq!(row.0, status);
    assert_eq!(row.1, attempt_count);
    assert_eq!(row.2.as_deref(), error_code);
    Ok(())
}

type TestSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn send_subscribe(socket: &mut TestSocket, conversation_id: Uuid) -> TestResult {
    socket
        .send(ClientMessage::Text(
            json!({
                "type": "subscribe",
                "request_id": Uuid::new_v4(),
                "conversation_id": conversation_id,
            })
            .to_string()
            .into(),
        ))
        .await?;
    Ok(())
}

async fn next_text(socket: &mut TestSocket) -> TestResult<String> {
    match next_message(socket).await? {
        ClientMessage::Text(payload) => Ok(payload.to_string()),
        other => {
            Err(io::Error::other(format!("expected a text WebSocket frame, got {other:?}")).into())
        }
    }
}

async fn next_message(socket: &mut TestSocket) -> TestResult<ClientMessage> {
    tokio::time::timeout(Duration::from_secs(3), socket.next())
        .await?
        .ok_or_else(|| io::Error::other("WebSocket ended before the expected recovery frame"))?
        .map_err(Into::into)
}

fn recovery_coordination_dir() -> TestResult<PathBuf> {
    let directory = env::var("JAMYE_TASK4B_RECOVERY_COORD_DIR").map_err(|_| {
        io::Error::other("JAMYE_TASK4B_RECOVERY_COORD_DIR is required for the ignored test")
    })?;
    let directory = PathBuf::from(directory);
    if !directory.is_dir() {
        return Err(io::Error::other("recovery coordination directory is absent").into());
    }
    Ok(directory)
}

fn write_marker(directory: &Path, marker: &str) -> TestResult {
    fs::write(directory.join(marker), b"ready")?;
    Ok(())
}

async fn wait_for_marker(directory: &Path, marker: &str) -> TestResult {
    let path = directory.join(marker);
    for _ in 0..600 {
        if path.is_file() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(io::Error::other(format!("timed out waiting for marker {marker}")).into())
}

fn guarded_redis_url() -> TestResult<String> {
    if env::var("JAMYE_ENVIRONMENT").as_deref() != Ok("test") {
        return Err(io::Error::other("recovery requires JAMYE_ENVIRONMENT=test").into());
    }
    let redis_url = env::var("REDIS_URL")
        .map_err(|_| io::Error::other("REDIS_URL is required for recovery"))?;
    let parsed = Url::parse(&redis_url)?;
    if parsed.scheme() != "redis"
        || !matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
    {
        return Err(io::Error::other("recovery accepts only a loopback redis:// URL").into());
    }
    Ok(redis_url)
}

struct StaticVerifier(AccessIdentity);

impl AccessTokenVerifier for StaticVerifier {
    fn verify(&self, token: &str) -> Result<AccessIdentity, AuthenticationError> {
        if token == TEST_BEARER {
            Ok(self.0.clone())
        } else {
            Err(AuthenticationError)
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TicketResponse {
    ticket: String,
    expires_at: String,
    contract_version: String,
}
