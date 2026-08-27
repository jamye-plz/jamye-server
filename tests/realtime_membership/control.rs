use std::{collections::HashSet, io, sync::Arc, time::Duration};

use axum::{
    Router,
    extract::{State, ws::WebSocketUpgrade},
    response::Response,
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use jamye_server::{
    adapters::redis::realtime_control::{
        RealtimeControlError, RealtimeControlWorker, RedisControlSubscriber, RedisRealtimeControl,
    },
    application::realtime::{RealtimeSession, membership_revocation::RealtimeControlIntent},
    ports::realtime::{ConversationAuthorizer, RealtimeClock, RealtimeFuture},
    transport::realtime::{
        CLOSE_MEMBERSHIP_REQUIRED, LocalRealtimeHub, RealtimeEvictionReason, SocketTiming,
        authorization::RealtimeControlConsumer, run_socket_with_runtime,
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

use crate::{
    TestResult,
    helpers::{
        create_group, guarded_redis_url, harness, insert_member, insert_user, worker_config,
    },
    postgres_support::TestDatabase,
};

#[tokio::test]
async fn durable_redis_control_evicts_two_nodes_before_exact_websocket_close() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "다중 노드 소유자").await?;
    let member_id = insert_user(&pool, "다중 노드 멤버").await?;
    let group = create_group(&fixture, owner_id, "다중 노드 그룹").await?;
    insert_member(&pool, group.id, member_id).await?;

    let redis = RedisRealtimeControl::new(&guarded_redis_url()?)?;
    let mut subscriber_a = redis.subscriber().await?;
    let mut subscriber_b = redis.subscriber().await?;
    let hub_a = LocalRealtimeHub::default();
    let hub_b = LocalRealtimeHub::default();
    let (mut socket_a, shutdown_a, server_a) = connect(websocket_state(
        hub_a.clone(),
        member_id,
        group.main_chatroom_id,
    )?)
    .await?;
    let (mut socket_b, shutdown_b, server_b) = connect(websocket_state(
        hub_b.clone(),
        member_id,
        group.main_chatroom_id,
    )?)
    .await?;
    subscribe(&mut socket_a, group.main_chatroom_id).await?;
    subscribe(&mut socket_b, group.main_chatroom_id).await?;
    assert_eq!(hub_a.registry_counts().await, (1, 1, 1));
    assert_eq!(hub_b.registry_counts().await, (1, 1, 1));

    let intent = fixture
        .revocations
        .remove_member(owner_id, group.id, member_id)
        .await?;
    let worker = RealtimeControlWorker::new(fixture.store.clone(), redis, worker_config())?;
    let report = worker.run_once().await?;
    assert_eq!(report.claimed, 1);
    assert_eq!(report.published, 1);

    let received_a = next_matching(&mut subscriber_a, intent.control_id()).await?;
    let received_b = next_matching(&mut subscriber_b, intent.control_id()).await?;
    let consumer_a = RealtimeControlConsumer::new(hub_a.clone(), fixture.store.clone());
    let consumer_b = RealtimeControlConsumer::new(hub_b.clone(), fixture.store.clone());
    assert_eq!(consumer_a.apply(&received_a).await?, 1);
    assert_eq!(consumer_b.apply(&received_b).await?, 1);

    // Registry entries are synchronously removed before the socket task sees its close signal.
    assert_eq!(hub_a.registry_counts().await, (0, 0, 0));
    assert_eq!(hub_b.registry_counts().await, (0, 0, 0));
    for close in [
        next_close(&mut socket_a).await?,
        next_close(&mut socket_b).await?,
    ] {
        assert_eq!(u16::from(close.code), CLOSE_MEMBERSHIP_REQUIRED);
        assert_eq!(close.reason.as_str(), "membership_revoked");
    }

    let status = sqlx::query_scalar::<_, String>("SELECT status FROM outbox_events WHERE id = $1")
        .bind(intent.control_id())
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "published");
    stop(shutdown_a, server_a).await?;
    stop(shutdown_b, server_b).await?;
    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn group_delete_evicts_every_local_group_connection_with_stable_reason() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "그룹 삭제 소유자").await?;
    let member_id = insert_user(&pool, "그룹 삭제 멤버").await?;
    let group = create_group(&fixture, owner_id, "삭제할 그룹").await?;
    insert_member(&pool, group.id, member_id).await?;

    let hub = LocalRealtimeHub::default();
    let mut owner = hub.register(owner_id).await;
    let mut member = hub.register(member_id).await;
    assert!(hub.subscribe(owner.socket_id, group.main_chatroom_id).await);
    assert!(
        hub.subscribe(member.socket_id, group.main_chatroom_id)
            .await
    );
    let intent = fixture.revocations.delete_group(owner_id, group.id).await?;
    let consumer = RealtimeControlConsumer::new(hub.clone(), fixture.store.clone());
    assert_eq!(consumer.apply(&intent).await?, 2);
    assert_eq!(hub.registry_counts().await, (0, 0, 0));

    for eviction in [owner.evictions.recv().await, member.evictions.recv().await] {
        let eviction = eviction.ok_or_else(|| io::Error::other("missing group eviction"))?;
        assert_eq!(eviction.reason, RealtimeEvictionReason::GroupDeleted);
        assert_eq!(eviction.reason.as_str(), "group_deleted");
    }
    assert_eq!(owner.outbound.recv().await, None);
    assert_eq!(member.outbound.recv().await, None);

    pool.close().await;
    database.dispose().await
}

async fn next_matching(
    subscriber: &mut RedisControlSubscriber,
    control_id: Uuid,
) -> TestResult<RealtimeControlIntent> {
    Ok(tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let control = subscriber
                .next_control()
                .await?
                .ok_or(RealtimeControlError::Unavailable)?;
            if control.control_id() == control_id {
                return Ok::<_, RealtimeControlError>(control);
            }
        }
    })
    .await??)
}

#[derive(Clone)]
struct WebSocketState {
    session: RealtimeSession,
    hub: LocalRealtimeHub,
    authorizer: Arc<dyn ConversationAuthorizer>,
    clock: Arc<dyn RealtimeClock>,
    timing: SocketTiming,
}

fn websocket_state(
    hub: LocalRealtimeHub,
    user_id: Uuid,
    conversation_id: Uuid,
) -> TestResult<WebSocketState> {
    Ok(WebSocketState {
        session: RealtimeSession {
            user_id,
            session_id: Uuid::new_v4(),
            contract_version: "1".to_owned(),
            access_token_expires_at: OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(30),
        },
        hub,
        authorizer: Arc::new(SetAuthorizer {
            allowed: [conversation_id].into_iter().collect(),
        }),
        clock: Arc::new(FixedClock(OffsetDateTime::UNIX_EPOCH)),
        timing: SocketTiming::new(Duration::from_secs(5), Duration::from_secs(5))
            .ok_or_else(|| io::Error::other("invalid task-6c socket timing"))?,
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

async fn subscribe(socket: &mut TestSocket, conversation_id: Uuid) -> TestResult {
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
    let frame: Value = serde_json::from_str(&next_text(socket).await?)?;
    assert_eq!(frame["type"], "subscribed");
    Ok(())
}

async fn next_text(socket: &mut TestSocket) -> TestResult<String> {
    match next_message(socket).await? {
        ClientMessage::Text(payload) => Ok(payload.to_string()),
        other => {
            Err(io::Error::other(format!("expected task-6c text frame, got {other:?}")).into())
        }
    }
}

async fn next_close(socket: &mut TestSocket) -> TestResult<CloseFrame> {
    match next_message(socket).await? {
        ClientMessage::Close(Some(frame)) => Ok(frame),
        other => {
            Err(io::Error::other(format!("expected task-6c close frame, got {other:?}")).into())
        }
    }
}

async fn next_message(socket: &mut TestSocket) -> TestResult<ClientMessage> {
    tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await?
        .ok_or_else(|| io::Error::other("task-6c WebSocket ended before expected frame"))?
        .map_err(Into::into)
}

async fn stop(
    shutdown: oneshot::Sender<()>,
    server: JoinHandle<Result<(), io::Error>>,
) -> TestResult {
    let _ = shutdown.send(());
    server.await??;
    Ok(())
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
