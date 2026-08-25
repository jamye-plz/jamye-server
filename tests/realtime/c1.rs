use std::{env, io, sync::Arc, time::Duration};

use axum::Router;
use futures_util::{SinkExt, StreamExt};
use jamye_server::{
    adapters::{
        postgres::{
            dev_fixtures::PostgresDevFixtureStore, messaging::PostgresMessagingRepository,
            realtime::PostgresRealtimeRepository, transactions::SqlxTransactionManager,
        },
        redis::realtime::{OsTicketCredentialSource, RedisRealtimeAdapter},
    },
    application::{
        messaging::MessagingService,
        realtime::{OutboxWorker, OutboxWorkerConfig, RealtimeTicketService, SystemClock},
    },
    dev_fixtures::{DevFixtureGuard, DevTokenCodec, SeededFixture},
    domain::messaging::{CanonicalMessage, DeltaItem, EventPage, MessageCreatedEvent},
    transport::{
        http::{
            dev_fixtures::{DevFixtureHttpState, router as dev_fixture_router},
            messaging::{MessagingHttpState, router as messaging_router},
            realtime::{RealtimeHttpState, router as realtime_router},
        },
        realtime::{CLOSE_REALTIME_AUTH_FAILED, LocalRealtimeHub},
    },
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::{net::TcpListener, sync::oneshot};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message as ClientMessage,
};
use url::Url;
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

#[tokio::test]
async fn dev_c1_flows_from_seed_to_rest_outbox_redis_websocket_and_delta() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let redis_url = guarded_redis_url()?;
    let guard = DevFixtureGuard::from_env()?;
    let codec = DevTokenCodec::ephemeral(guard);
    let fixture_state =
        DevFixtureHttpState::new(Arc::new(PostgresDevFixtureStore::new(pool.clone())), codec);
    let auth = fixture_state.auth_state();
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
        .merge(dev_fixture_router(fixture_state))
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

    let seed_response = client
        .post(format!("{base_url}/__dev/fixtures/seed"))
        .send()
        .await?;
    assert_eq!(seed_response.status(), reqwest::StatusCode::CREATED);
    let fixture: SeededFixture = serde_json::from_slice(&seed_response.bytes().await?)?;

    let phase_one = delta_page(&client, &base_url, &fixture, None, 1).await?;
    assert!(phase_one.items.is_empty());
    assert_eq!(phase_one.next_cursor, None);

    let ticket_response = client
        .post(format!("{base_url}/api/v1/realtime/tickets"))
        .bearer_auth(&fixture.access_token)
        .header("x-jamye-contract-version", "1")
        .send()
        .await?;
    assert_eq!(ticket_response.status(), reqwest::StatusCode::CREATED);
    let ticket: TicketResponse = serde_json::from_slice(&ticket_response.bytes().await?)?;
    assert_eq!(ticket.contract_version, "1");
    assert!(!ticket.expires_at.is_empty());

    let mut subscriber = redis_realtime.event_subscriber().await?;
    let forward_hub = hub.clone();
    let expected_conversation_id = fixture.chatroom_id;
    let forwarder = tokio::spawn(async move {
        let event = loop {
            let event = subscriber
                .next_event()
                .await
                .map_err(io::Error::other)?
                .ok_or_else(|| io::Error::other("Redis event subscriber ended"))?;
            if event.conversation_id == expected_conversation_id {
                break event;
            }
        };
        let conversation_id = event.conversation_id;
        let payload = serde_json::to_string(&event).map_err(io::Error::other)?;
        let recipients = forward_hub.publish(conversation_id, payload).await;
        if recipients != 1 {
            return Err(io::Error::other(format!(
                "expected one WebSocket recipient, got {recipients}"
            )));
        }
        Ok::<(), io::Error>(())
    });

    let ws_url = format!("ws://{address}/api/v1/realtime/ws?ticket={}", ticket.ticket);
    let (mut socket, _) = connect_async(ws_url.as_str()).await?;
    assert_auth_failed_close(ws_url.as_str()).await?;
    let invalid_ws_url = format!("ws://{address}/api/v1/realtime/ws?ticket=invalid");
    assert_auth_failed_close(&invalid_ws_url).await?;
    let subscribe_request_id = Uuid::new_v4();
    socket
        .send(ClientMessage::Text(
            json!({
                "type": "subscribe",
                "request_id": subscribe_request_id,
                "conversation_id": fixture.chatroom_id,
            })
            .to_string()
            .into(),
        ))
        .await?;
    let subscribed: Value = serde_json::from_str(&next_text(&mut socket).await?)?;
    assert_eq!(subscribed["type"], "subscribed");
    assert_eq!(subscribed["request_id"], subscribe_request_id.to_string());
    assert_eq!(
        subscribed["conversation_id"],
        fixture.chatroom_id.to_string()
    );

    let client_msg_id = Uuid::new_v4();
    let canonical = post_message(
        &client,
        &base_url,
        &fixture,
        client_msg_id,
        reqwest::StatusCode::CREATED,
    )
    .await?;
    let worker = OutboxWorker::new(
        postgres_realtime,
        redis_realtime,
        OutboxWorkerConfig {
            claim_owner: "task-4b-c1".to_owned(),
            batch_size: 10,
            lease_duration: Duration::from_secs(2),
            publish_timeout: Duration::from_millis(500),
            lease_safety_margin: Duration::from_millis(100),
            retry_delay: Duration::from_millis(10),
            poll_interval: Duration::from_millis(10),
            max_attempts: 3,
        },
    )?;
    let report = worker.run_once().await?;
    assert_eq!(report.claimed, 1);
    assert_eq!(report.published, 1);
    forwarder.await??;

    let realtime_event: MessageCreatedEvent = serde_json::from_str(&next_text(&mut socket).await?)?;
    assert_eq!(realtime_event.data, canonical);
    assert_eq!(realtime_event.conversation_id, fixture.chatroom_id);
    assert_eq!(realtime_event.version, 1);

    assert_persisted_event_matches(&pool, &realtime_event, &canonical).await?;
    let phase_two = delta_page(&client, &base_url, &fixture, None, 1).await?;
    assert_eq!(phase_two.items.len(), 1);
    assert_eq!(phase_two.items[0], DeltaItem::Known(realtime_event.clone()));
    assert_eq!(phase_two.next_cursor, None);
    let terminal = delta_page(
        &client,
        &base_url,
        &fixture,
        Some(&realtime_event.cursor),
        1,
    )
    .await?;
    assert!(terminal.items.is_empty());
    assert_eq!(terminal.next_cursor, None);

    let retried = post_message(
        &client,
        &base_url,
        &fixture,
        client_msg_id,
        reqwest::StatusCode::OK,
    )
    .await?;
    assert_eq!(retried, canonical);
    assert_single_commit(&pool, fixture.user_id, client_msg_id).await?;

    socket.close(None).await?;
    let _ = shutdown_sender.send(());
    server.await??;
    pool.close().await;
    database.dispose().await
}

async fn post_message(
    client: &reqwest::Client,
    base_url: &str,
    fixture: &SeededFixture,
    client_msg_id: Uuid,
    expected_status: reqwest::StatusCode,
) -> TestResult<CanonicalMessage> {
    let response = client
        .post(format!(
            "{base_url}/api/v1/chatrooms/{}/messages",
            fixture.chatroom_id
        ))
        .bearer_auth(&fixture.access_token)
        .header("content-type", "application/json")
        .header("idempotency-key", client_msg_id.to_string())
        .body(
            json!({
                "client_msg_id": client_msg_id,
                "body": "task-4b c1 exact text",
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
    fixture: &SeededFixture,
    after: Option<&str>,
    limit: u32,
) -> TestResult<EventPage> {
    let query = after.map_or_else(
        || format!("limit={limit}"),
        |after| format!("after={after}&limit={limit}"),
    );
    let response = client
        .get(format!(
            "{base_url}/api/v1/conversations/{}/events?{query}",
            fixture.chatroom_id
        ))
        .bearer_auth(&fixture.access_token)
        .header("x-jamye-contract-version", "1")
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    Ok(serde_json::from_slice(&response.bytes().await?)?)
}

async fn assert_persisted_event_matches(
    pool: &PgPool,
    event: &MessageCreatedEvent,
    canonical: &CanonicalMessage,
) -> TestResult {
    let row = sqlx::query_as::<_, (i64, Value, String)>(
        "SELECT e.cursor, e.payload, o.status \
         FROM conversation_events e \
         JOIN outbox_events o ON o.conversation_event_id = e.id \
         WHERE e.id = $1",
    )
    .bind(event.event_id)
    .fetch_one(pool)
    .await?;
    assert_eq!(row.0.to_string(), event.cursor);
    assert_eq!(
        serde_json::from_value::<CanonicalMessage>(row.1)?,
        *canonical
    );
    assert_eq!(row.2, "published");
    Ok(())
}

async fn assert_single_commit(pool: &PgPool, user_id: Uuid, client_msg_id: Uuid) -> TestResult {
    let row = sqlx::query_as::<_, (i64, i64, i64)>(
        "SELECT \
           (SELECT COUNT(*) FROM messages WHERE sender_id = $1 AND client_msg_id = $2), \
           (SELECT COUNT(*) FROM conversation_events e \
              JOIN messages m ON m.chatroom_id = e.conversation_id \
             WHERE m.sender_id = $1 AND m.client_msg_id = $2), \
           (SELECT COUNT(*) FROM outbox_events o \
              JOIN conversation_events e ON e.id = o.conversation_event_id \
              JOIN messages m ON m.chatroom_id = e.conversation_id \
             WHERE m.sender_id = $1 AND m.client_msg_id = $2)",
    )
    .bind(user_id)
    .bind(client_msg_id)
    .fetch_one(pool)
    .await?;
    assert_eq!(row, (1, 1, 1));
    Ok(())
}

async fn next_text(
    socket: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
) -> TestResult<String> {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await?
            .ok_or_else(|| io::Error::other("WebSocket ended before the expected frame"))??;
        match message {
            ClientMessage::Text(payload) => return Ok(payload.to_string()),
            ClientMessage::Ping(payload) => socket.send(ClientMessage::Pong(payload)).await?,
            ClientMessage::Close(frame) => {
                return Err(io::Error::other(format!(
                    "WebSocket closed before the expected frame: {frame:?}"
                ))
                .into());
            }
            ClientMessage::Binary(_) | ClientMessage::Pong(_) | ClientMessage::Frame(_) => {}
        }
    }
}

async fn assert_auth_failed_close(ws_url: &str) -> TestResult {
    let (mut socket, _) = connect_async(ws_url).await?;
    let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await?
        .ok_or_else(|| io::Error::other("unauthenticated WebSocket ended without close"))??;
    match message {
        ClientMessage::Close(Some(frame)) => {
            assert_eq!(u16::from(frame.code), CLOSE_REALTIME_AUTH_FAILED);
            assert_eq!(frame.reason.as_str(), "realtime_auth_failed");
            Ok(())
        }
        other => Err(io::Error::other(format!(
            "expected realtime_auth_failed close, got {other:?}"
        ))
        .into()),
    }
}

fn guarded_redis_url() -> TestResult<String> {
    if env::var("JAMYE_ENVIRONMENT").as_deref() != Ok("test") {
        return Err(io::Error::other("C1 requires JAMYE_ENVIRONMENT=test").into());
    }
    let redis_url =
        env::var("REDIS_URL").map_err(|_| io::Error::other("REDIS_URL is required for C1"))?;
    let parsed = Url::parse(&redis_url)?;
    if parsed.scheme() != "redis"
        || !matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
    {
        return Err(io::Error::other("C1 accepts only a loopback redis:// URL").into());
    }
    Ok(redis_url)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TicketResponse {
    ticket: String,
    expires_at: String,
    contract_version: String,
}
