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
    transport::http::realtime::limit_websocket_upgrade,
    transport::realtime::{
        CLOSE_GOING_AWAY, CLOSE_MEMBERSHIP_REQUIRED, CLOSE_MESSAGE_TOO_BIG, CLOSE_PROTOCOL_ERROR,
        CLOSE_REALTIME_AUTH_EXPIRED, CLOSE_TRY_AGAIN_LATER, LocalRealtimeHub, SocketTiming,
        run_socket_with_runtime,
    },
};
use serde_json::{Value, json};
use time::OffsetDateTime;
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{
        Message as ClientMessage,
        protocol::{
            CloseFrame,
            frame::{
                Frame,
                coding::{Data, OpCode},
            },
        },
    },
};
use uuid::Uuid;

use crate::TestResult;

#[tokio::test]
async fn text_message_of_4096_bytes_is_accepted() -> TestResult {
    let state = default_limit_state(LocalRealtimeHub::default())?;
    let (mut socket, shutdown, server) = connect(state).await?;
    socket
        .send(ClientMessage::Text(padded_ping(4096).into()))
        .await?;
    let pong: Value = serde_json::from_str(&next_text(&mut socket).await?)?;
    assert_eq!(pong, json!({"type": "pong", "nonce": "limits"}));
    drop(socket);
    stop(shutdown, server).await
}

#[tokio::test]
async fn text_message_over_4096_closes_1009() -> TestResult {
    let hub = LocalRealtimeHub::default();
    let (mut socket, shutdown, server) = connect(default_limit_state(hub.clone())?).await?;
    socket
        .send(ClientMessage::Text(padded_ping(4097).into()))
        .await?;
    let close = next_close(&mut socket).await?;
    assert_eq!(u16::from(close.code), CLOSE_MESSAGE_TOO_BIG);
    assert_eq!(close.reason.as_str(), "message_too_big");
    assert_eq!(hub.registry_counts().await, (0, 0, 0));
    stop(shutdown, server).await
}

#[tokio::test]
async fn fragmented_message_over_4096_closes_1009() -> TestResult {
    let hub = LocalRealtimeHub::default();
    let (mut socket, shutdown, server) = connect(default_limit_state(hub.clone())?).await?;
    let message = padded_ping(4097).into_bytes();
    socket
        .send(ClientMessage::Frame(Frame::message(
            message[..2048].to_vec(),
            OpCode::Data(Data::Text),
            false,
        )))
        .await?;
    socket
        .send(ClientMessage::Frame(Frame::message(
            message[2048..].to_vec(),
            OpCode::Data(Data::Continue),
            true,
        )))
        .await?;
    let close = next_close(&mut socket).await?;
    assert_eq!(u16::from(close.code), CLOSE_MESSAGE_TOO_BIG);
    assert_eq!(close.reason.as_str(), "message_too_big");
    assert_eq!(hub.registry_counts().await, (0, 0, 0));
    stop(shutdown, server).await
}

#[tokio::test]
async fn invalid_continuation_closes_4400() -> TestResult {
    let (mut socket, shutdown, server) =
        connect(default_limit_state(LocalRealtimeHub::default())?).await?;
    socket
        .send(ClientMessage::Frame(Frame::message(
            b"invalid".to_vec(),
            OpCode::Data(Data::Continue),
            true,
        )))
        .await?;
    let close = next_close(&mut socket).await?;
    assert_eq!(u16::from(close.code), CLOSE_PROTOCOL_ERROR);
    assert_eq!(close.reason.as_str(), "protocol_error");
    stop(shutdown, server).await
}

#[tokio::test]
async fn eighth_socket_is_accepted() -> TestResult {
    let hub = LocalRealtimeHub::default();
    let state = default_limit_state(hub.clone())?;
    let mut connections = Vec::new();
    for _ in 0..8 {
        let (mut socket, shutdown, server) = connect(state.clone()).await?;
        assert_ping(&mut socket).await?;
        connections.push((socket, shutdown, server));
    }
    assert_eq!(hub.registry_counts().await, (8, 8, 0));
    for (socket, shutdown, server) in connections {
        drop(socket);
        stop(shutdown, server).await?;
    }
    Ok(())
}

#[tokio::test]
async fn ninth_socket_per_user_closes_1013() -> TestResult {
    let hub = LocalRealtimeHub::default();
    let state = default_limit_state(hub.clone())?;
    let mut connections = Vec::new();
    for _ in 0..8 {
        let (mut socket, shutdown, server) = connect(state.clone()).await?;
        assert_ping(&mut socket).await?;
        connections.push((socket, shutdown, server));
    }
    let (mut rejected, rejected_shutdown, rejected_server) = connect(state.clone()).await?;
    let close = next_close(&mut rejected).await?;
    assert_eq!(u16::from(close.code), CLOSE_TRY_AGAIN_LATER);
    assert_eq!(close.reason.as_str(), "socket_limit");
    assert_eq!(hub.registry_counts().await, (8, 8, 0));
    for (socket, _, _) in &mut connections {
        assert_ping(socket).await?;
    }
    let (mut other_user, other_shutdown, other_server) =
        connect(default_limit_state(hub.clone())?).await?;
    assert_ping(&mut other_user).await?;
    assert_eq!(hub.registry_counts().await, (9, 9, 0));
    drop(other_user);
    stop(other_shutdown, other_server).await?;
    stop(rejected_shutdown, rejected_server).await?;
    for (socket, shutdown, server) in connections {
        drop(socket);
        stop(shutdown, server).await?;
    }
    Ok(())
}

#[tokio::test]
async fn slot_released_on_disconnect_expiry_and_shutdown() -> TestResult {
    // Shutdown means a live server-side session ends; process exit drops the in-memory registry.
    for ending in ["close", "drop", "expiry", "heartbeat"] {
        let hub = LocalRealtimeHub::default();
        let mut state = default_limit_state(hub.clone())?;
        if ending == "expiry" {
            state.session.access_token_expires_at =
                OffsetDateTime::UNIX_EPOCH + time::Duration::milliseconds(500);
        }
        if ending == "heartbeat" {
            state.timing = SocketTiming::new(
                Duration::from_millis(400),
                Duration::from_millis(100),
                Duration::from_secs(1),
            )
            .ok_or_else(|| io::Error::other("invalid heartbeat timing"))?;
        }
        let mut held = Vec::new();
        for _ in 0..7 {
            held.push(hub.register(state.session.user_id).await?);
        }
        let (mut socket, shutdown, server) = connect(state.clone()).await?;
        assert_ping(&mut socket).await?;
        assert_eq!(hub.registry_counts().await, (8, 8, 0));
        match ending {
            "close" => socket.close(None).await?,
            "drop" => drop(socket),
            "expiry" => assert_eq!(
                u16::from(next_close(&mut socket).await?.code),
                CLOSE_REALTIME_AUTH_EXPIRED
            ),
            "heartbeat" => assert_eq!(
                u16::from(next_close(&mut socket).await?.code),
                CLOSE_PROTOCOL_ERROR
            ),
            _ => unreachable!("unknown session ending"),
        }
        wait_for_socket_count(&hub, 7).await?;
        state.session.access_token_expires_at =
            OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(30);
        state.timing = SocketTiming::new(
            Duration::from_secs(5),
            Duration::from_secs(5),
            Duration::from_secs(1),
        )
        .ok_or_else(|| io::Error::other("invalid replacement timing"))?;
        let (mut replacement, replacement_shutdown, replacement_server) = connect(state).await?;
        assert_ping(&mut replacement).await?;
        assert_eq!(hub.registry_counts().await, (8, 8, 0));
        drop(replacement);
        stop(replacement_shutdown, replacement_server).await?;
        stop(shutdown, server).await?;
        for connection in held {
            assert!(hub.cleanup(connection.socket_id).await);
        }
        wait_for_socket_count(&hub, 0).await?;
    }
    Ok(())
}

#[tokio::test]
async fn shutdown_closes_existing_and_new_sockets_and_drains_registry() -> TestResult {
    let hub = LocalRealtimeHub::default();
    let state = default_limit_state(hub.clone())?;
    let (mut socket, shutdown, server) = connect(state.clone()).await?;
    assert_ping(&mut socket).await?;
    assert_eq!(hub.registry_counts().await, (1, 1, 0));

    hub.shutdown().await;
    let close = next_close(&mut socket).await?;
    assert_eq!(u16::from(close.code), CLOSE_GOING_AWAY);
    assert_eq!(close.reason.as_str(), "server_shutdown");
    assert!(hub.wait_for_drain(Duration::from_secs(2)).await);
    assert_eq!(hub.registry_counts().await, (0, 0, 0));

    let (mut new_socket, new_shutdown, new_server) = connect(state).await?;
    let close = next_close(&mut new_socket).await?;
    assert_eq!(u16::from(close.code), CLOSE_GOING_AWAY);
    assert_eq!(close.reason.as_str(), "server_shutdown");
    assert_eq!(hub.registry_counts().await, (0, 0, 0));

    stop(new_shutdown, new_server).await?;
    stop(shutdown, server).await
}

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

#[tokio::test]
async fn previous_contract_socket_drops_unknown_realtime_discriminants_without_closing()
-> TestResult {
    let conversation_id = Uuid::new_v4();
    let hub = LocalRealtimeHub::default();
    let state = websocket_state_with_contract(
        hub.clone(),
        [conversation_id],
        OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(30),
        Duration::from_secs(5),
        Duration::from_secs(5),
        "1",
    )?;
    let (mut socket, shutdown, server) = connect(state).await?;
    send_subscribe(&mut socket, Uuid::new_v4(), conversation_id).await?;
    let subscribed: Value = serde_json::from_str(&next_text(&mut socket).await?)?;
    assert_eq!(subscribed["type"], "subscribed");

    let payload = json!({
        "version": 1,
        "type": "message.deleted",
        "event_id": Uuid::new_v4(),
        "conversation_id": conversation_id,
        "cursor": "1",
        "occurred_at": "1970-01-01T00:00:00Z",
        "message_id": Uuid::new_v4(),
    })
    .to_string();
    assert_eq!(hub.publish(conversation_id, payload).await, 1);
    assert_no_message(&mut socket).await?;

    let topic_payload = json!({
        "version": 1,
        "type": "topic.deleted",
        "event_id": Uuid::new_v4(),
        "conversation_id": conversation_id,
        "cursor": "2",
        "occurred_at": "1970-01-01T00:00:00Z",
        "data": {
            "topic_id": Uuid::new_v4(),
            "topic_chatroom_id": Uuid::new_v4(),
            "group_id": Uuid::new_v4(),
            "deleted_at": "1970-01-01T00:00:00Z",
            "deleted_by": Uuid::new_v4(),
            "announcement_message_id": null,
        },
    })
    .to_string();
    assert_eq!(hub.publish(conversation_id, topic_payload).await, 1);
    assert_no_message(&mut socket).await?;

    socket
        .send(ClientMessage::Text(
            json!({"type": "ping", "nonce": "still-open"})
                .to_string()
                .into(),
        ))
        .await?;
    let pong: Value = serde_json::from_str(&next_text(&mut socket).await?)?;
    assert_eq!(pong, json!({"type": "pong", "nonce": "still-open"}));
    stop(shutdown, server).await
}

#[tokio::test]
async fn current_contract_socket_receives_topic_deleted_on_subscribed_main_conversation()
-> TestResult {
    let conversation_id = Uuid::new_v4();
    let topic_chatroom_id = Uuid::new_v4();
    let topic_id = Uuid::new_v4();
    let group_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let hub = LocalRealtimeHub::default();
    let state = websocket_state(
        hub.clone(),
        [conversation_id],
        OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(30),
        Duration::from_secs(5),
        Duration::from_secs(5),
    )?;
    let (mut socket, shutdown, server) = connect(state).await?;
    send_subscribe(&mut socket, Uuid::new_v4(), conversation_id).await?;
    let subscribed: Value = serde_json::from_str(&next_text(&mut socket).await?)?;
    assert_eq!(subscribed["type"], "subscribed");

    let payload = json!({
        "version": 1,
        "type": "topic.deleted",
        "event_id": event_id,
        "conversation_id": conversation_id,
        "cursor": "1",
        "occurred_at": "1970-01-01T00:00:00Z",
        "data": {
            "topic_id": topic_id,
            "topic_chatroom_id": topic_chatroom_id,
            "group_id": group_id,
            "deleted_at": "1970-01-01T00:00:00Z",
            "deleted_by": Uuid::new_v4(),
            "announcement_message_id": null,
        },
    })
    .to_string();
    assert_eq!(hub.publish(conversation_id, payload).await, 1);

    let frame: Value = serde_json::from_str(&next_text(&mut socket).await?)?;
    assert_eq!(frame["type"], "topic.deleted");
    assert_eq!(frame["event_id"], event_id.to_string());
    assert_eq!(frame["conversation_id"], conversation_id.to_string());
    assert_eq!(frame["data"]["topic_id"], topic_id.to_string());
    assert_eq!(
        frame["data"]["topic_chatroom_id"],
        topic_chatroom_id.to_string()
    );
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

fn default_limit_state(hub: LocalRealtimeHub) -> TestResult<WebSocketState> {
    websocket_state(
        hub,
        [],
        OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(30),
        Duration::from_secs(5),
        Duration::from_secs(5),
    )
}

fn padded_ping(length: usize) -> String {
    let mut payload = json!({"type": "ping", "nonce": "limits"}).to_string();
    payload.push_str(&" ".repeat(length - payload.len()));
    payload
}

async fn assert_ping(socket: &mut TestSocket) -> TestResult {
    socket
        .send(ClientMessage::Text(padded_ping(40).into()))
        .await?;
    let pong: Value = serde_json::from_str(&next_text(socket).await?)?;
    assert_eq!(pong, json!({"type": "pong", "nonce": "limits"}));
    Ok(())
}

async fn wait_for_socket_count(hub: &LocalRealtimeHub, expected: usize) -> TestResult {
    tokio::time::timeout(Duration::from_secs(2), async {
        while hub.registry_counts().await.0 != expected {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    Ok(())
}

fn websocket_state<const N: usize>(
    hub: LocalRealtimeHub,
    allowed_conversations: [Uuid; N],
    access_token_expires_at: OffsetDateTime,
    ping_interval: Duration,
    pong_deadline: Duration,
) -> TestResult<WebSocketState> {
    websocket_state_with_contract(
        hub,
        allowed_conversations,
        access_token_expires_at,
        ping_interval,
        pong_deadline,
        "2",
    )
}

fn websocket_state_with_contract<const N: usize>(
    hub: LocalRealtimeHub,
    allowed_conversations: [Uuid; N],
    access_token_expires_at: OffsetDateTime,
    ping_interval: Duration,
    pong_deadline: Duration,
    contract_version: &str,
) -> TestResult<WebSocketState> {
    Ok(WebSocketState {
        session: RealtimeSession {
            user_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            contract_version: contract_version.to_owned(),
            access_token_expires_at,
        },
        hub,
        authorizer: Arc::new(SetAuthorizer {
            allowed: allowed_conversations.into_iter().collect(),
        }),
        clock: Arc::new(FixedClock(OffsetDateTime::UNIX_EPOCH)),
        timing: SocketTiming::new(ping_interval, pong_deadline, Duration::from_secs(1))
            .ok_or_else(|| io::Error::other("invalid test socket timing"))?,
    })
}

async fn websocket(State(state): State<WebSocketState>, ws: WebSocketUpgrade) -> Response {
    limit_websocket_upgrade(ws).on_upgrade(move |socket| {
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

async fn assert_no_message(socket: &mut TestSocket) -> TestResult {
    match tokio::time::timeout(Duration::from_millis(200), socket.next()).await {
        Err(_) => Ok(()),
        Ok(Some(Ok(message))) => Err(io::Error::other(format!(
            "unexpected filtered WebSocket frame: {message:?}"
        ))
        .into()),
        Ok(Some(Err(error))) => Err(error.into()),
        Ok(None) => Err(io::Error::other("WebSocket closed while waiting for no frame").into()),
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
