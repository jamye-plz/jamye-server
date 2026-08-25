use std::{collections::HashSet, io, sync::Arc, time::Duration};

use axum::{
    Router,
    extract::{State, ws::WebSocketUpgrade},
    response::Response,
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use jamye_server::{
    application::realtime::RealtimeSession,
    ports::realtime::{ConversationAuthorizer, RealtimeClock, RealtimeFuture},
    transport::realtime::{
        CLOSE_MEMBERSHIP_REQUIRED, CLOSE_PROTOCOL_ERROR, CLOSE_REALTIME_AUTH_EXPIRED,
        LocalRealtimeHub, SocketTiming, run_socket_with_runtime,
    },
};
use serde_json::{Value, json};
use time::OffsetDateTime;
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message as ClientMessage, protocol::CloseFrame},
};
use uuid::Uuid;

use crate::TestResult;

#[tokio::test]
async fn denied_subscribe_cleans_prior_membership_and_closes_without_data_or_error() -> TestResult {
    let allowed_conversation = Uuid::new_v4();
    let denied_conversation = Uuid::new_v4();
    let hub = LocalRealtimeHub::default();
    let state = websocket_state(
        hub.clone(),
        [allowed_conversation],
        OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(30),
        Duration::from_secs(5),
        Duration::from_secs(5),
    )?;
    let (mut socket, shutdown, server) = connect(state).await?;

    let subscribe_id = Uuid::new_v4();
    send_subscribe(&mut socket, subscribe_id, allowed_conversation).await?;
    let subscribed: Value = serde_json::from_str(&next_text(&mut socket).await?)?;
    assert_eq!(subscribed["type"], "subscribed");
    assert_eq!(subscribed["request_id"], subscribe_id.to_string());
    assert_eq!(hub.registry_counts().await, (1, 1, 1));

    socket
        .send(ClientMessage::Text(
            json!({"type": "ping", "nonce": "task-4b"})
                .to_string()
                .into(),
        ))
        .await?;
    let pong: Value = serde_json::from_str(&next_text(&mut socket).await?)?;
    assert_eq!(pong, json!({"type": "pong", "nonce": "task-4b"}));

    let unsubscribe_id = Uuid::new_v4();
    send_unsubscribe(&mut socket, unsubscribe_id, allowed_conversation).await?;
    let unsubscribed: Value = serde_json::from_str(&next_text(&mut socket).await?)?;
    assert_eq!(unsubscribed["type"], "unsubscribed");
    assert_eq!(unsubscribed["request_id"], unsubscribe_id.to_string());
    assert_eq!(hub.registry_counts().await, (1, 1, 0));
    send_subscribe(&mut socket, Uuid::new_v4(), allowed_conversation).await?;
    let resubscribed: Value = serde_json::from_str(&next_text(&mut socket).await?)?;
    assert_eq!(resubscribed["type"], "subscribed");
    assert_eq!(hub.registry_counts().await, (1, 1, 1));

    send_subscribe(&mut socket, Uuid::new_v4(), denied_conversation).await?;
    let close = next_close(&mut socket).await?;
    assert_eq!(u16::from(close.code), CLOSE_MEMBERSHIP_REQUIRED);
    assert_eq!(close.reason.as_str(), "membership_required");
    assert_eq!(hub.registry_counts().await, (0, 0, 0));
    assert_eq!(
        hub.publish(allowed_conversation, "late".to_owned()).await,
        0
    );

    stop(shutdown, server).await
}

#[tokio::test]
async fn controlled_access_expiry_cleans_registry_before_exact_4401_close() -> TestResult {
    let conversation_id = Uuid::new_v4();
    let hub = LocalRealtimeHub::default();
    let state = websocket_state(
        hub.clone(),
        [conversation_id],
        OffsetDateTime::UNIX_EPOCH + time::Duration::milliseconds(200),
        Duration::from_secs(5),
        Duration::from_secs(5),
    )?;
    let (mut socket, shutdown, server) = connect(state).await?;
    send_subscribe(&mut socket, Uuid::new_v4(), conversation_id).await?;
    let subscribed: Value = serde_json::from_str(&next_text(&mut socket).await?)?;
    assert_eq!(subscribed["type"], "subscribed");

    let close = next_close(&mut socket).await?;
    assert_eq!(u16::from(close.code), CLOSE_REALTIME_AUTH_EXPIRED);
    assert_eq!(close.reason.as_str(), "realtime_auth_expired");
    assert_eq!(hub.registry_counts().await, (0, 0, 0));
    assert_eq!(hub.publish(conversation_id, "late".to_owned()).await, 0);

    stop(shutdown, server).await
}

#[tokio::test]
async fn missing_application_ping_reaches_the_protocol_heartbeat_timeout() -> TestResult {
    let hub = LocalRealtimeHub::default();
    let state = websocket_state(
        hub.clone(),
        [],
        OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(30),
        Duration::from_millis(25),
        Duration::from_millis(10),
    )?;
    let (mut socket, shutdown, server) = connect(state).await?;
    let close = next_close(&mut socket).await?;
    assert_eq!(u16::from(close.code), CLOSE_PROTOCOL_ERROR);
    assert_eq!(close.reason.as_str(), "heartbeat_timeout");
    assert_eq!(hub.registry_counts().await, (0, 0, 0));
    stop(shutdown, server).await
}

#[derive(Clone)]
struct WebSocketState {
    session: RealtimeSession,
    hub: LocalRealtimeHub,
    authorizer: Arc<dyn ConversationAuthorizer>,
    clock: Arc<dyn RealtimeClock>,
    timing: SocketTiming,
}

fn websocket_state<const N: usize>(
    hub: LocalRealtimeHub,
    allowed_conversations: [Uuid; N],
    access_token_expires_at: OffsetDateTime,
    ping_interval: Duration,
    pong_deadline: Duration,
) -> TestResult<WebSocketState> {
    Ok(WebSocketState {
        session: RealtimeSession {
            user_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            contract_version: "1".to_owned(),
            access_token_expires_at,
        },
        hub,
        authorizer: Arc::new(SetAuthorizer {
            allowed: allowed_conversations.into_iter().collect(),
        }),
        clock: Arc::new(FixedClock(OffsetDateTime::UNIX_EPOCH)),
        timing: SocketTiming::new(ping_interval, pong_deadline)
            .ok_or_else(|| io::Error::other("invalid test socket timing"))?,
    })
}

async fn websocket(State(state): State<WebSocketState>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| {
        run_socket_with_runtime(
            socket,
            state.session,
            state.hub,
            state.authorizer,
            state.clock,
            state.timing,
        )
    })
}

type TestSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(
    state: WebSocketState,
) -> TestResult<(
    TestSocket,
    oneshot::Sender<()>,
    JoinHandle<Result<(), io::Error>>,
)> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;
    let router = Router::new().route("/ws", get(websocket)).with_state(state);
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                let _ = shutdown_receiver.await;
            })
            .await
    });
    let (socket, _) = connect_async(format!("ws://{address}/ws")).await?;
    Ok((socket, shutdown_sender, server))
}

async fn stop(
    shutdown: oneshot::Sender<()>,
    server: JoinHandle<Result<(), io::Error>>,
) -> TestResult {
    let _ = shutdown.send(());
    server.await??;
    Ok(())
}

async fn send_subscribe(
    socket: &mut TestSocket,
    request_id: Uuid,
    conversation_id: Uuid,
) -> TestResult {
    socket
        .send(ClientMessage::Text(
            json!({
                "type": "subscribe",
                "request_id": request_id,
                "conversation_id": conversation_id,
            })
            .to_string()
            .into(),
        ))
        .await?;
    Ok(())
}

async fn send_unsubscribe(
    socket: &mut TestSocket,
    request_id: Uuid,
    conversation_id: Uuid,
) -> TestResult {
    socket
        .send(ClientMessage::Text(
            json!({
                "type": "unsubscribe",
                "request_id": request_id,
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

async fn next_close(socket: &mut TestSocket) -> TestResult<CloseFrame> {
    match next_message(socket).await? {
        ClientMessage::Close(Some(frame)) => Ok(frame),
        other => {
            Err(io::Error::other(format!("expected a close WebSocket frame, got {other:?}")).into())
        }
    }
}

async fn next_message(socket: &mut TestSocket) -> TestResult<ClientMessage> {
    tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await?
        .ok_or_else(|| io::Error::other("WebSocket ended before the expected frame"))?
        .map_err(Into::into)
}

struct SetAuthorizer {
    allowed: HashSet<Uuid>,
}

impl ConversationAuthorizer for SetAuthorizer {
    fn is_authorized(&self, _user_id: Uuid, conversation_id: Uuid) -> RealtimeFuture<'_, bool> {
        let allowed = self.allowed.contains(&conversation_id);
        Box::pin(async move { Ok(allowed) })
    }
}

struct FixedClock(OffsetDateTime);

impl RealtimeClock for FixedClock {
    fn now(&self) -> OffsetDateTime {
        self.0
    }
}
