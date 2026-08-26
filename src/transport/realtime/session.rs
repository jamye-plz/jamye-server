use std::{sync::Arc, time::Duration};

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use serde::{Deserialize, Serialize};
use tokio::time::Instant;
use uuid::Uuid;

use crate::{
    application::realtime::{RealtimeSession, SystemClock},
    ports::realtime::{ConversationAuthorizer, RealtimeClock},
    transport::realtime::LocalRealtimeHub,
};

pub const CLOSE_MEMBERSHIP_REQUIRED: u16 = 4001;
pub const CLOSE_PROTOCOL_ERROR: u16 = 4400;
pub const CLOSE_REALTIME_AUTH_FAILED: u16 = 4401;
pub const CLOSE_REALTIME_AUTH_EXPIRED: u16 = 4401;
pub const CLOSE_INTERNAL_ERROR: u16 = 1011;
const CLIENT_PING_INTERVAL_SECONDS: u64 = 25;
const PONG_DEADLINE_SECONDS: u64 = 10;
const HEARTBEAT_TIMEOUT: Duration =
    Duration::from_secs(CLIENT_PING_INTERVAL_SECONDS + PONG_DEADLINE_SECONDS);
const MAX_NONCE_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SocketTiming {
    heartbeat_timeout: Duration,
}

impl SocketTiming {
    pub fn new(client_ping_interval: Duration, pong_deadline: Duration) -> Option<Self> {
        if client_ping_interval.is_zero() || pong_deadline.is_zero() {
            return None;
        }
        Some(Self {
            heartbeat_timeout: client_ping_interval.checked_add(pong_deadline)?,
        })
    }

    fn protocol() -> Self {
        Self {
            heartbeat_timeout: HEARTBEAT_TIMEOUT,
        }
    }

    fn heartbeat_timeout(self) -> Duration {
        self.heartbeat_timeout
    }
}

pub async fn run_socket(
    socket: WebSocket,
    session: RealtimeSession,
    hub: LocalRealtimeHub,
    authorizer: Arc<dyn ConversationAuthorizer>,
) {
    run_socket_with_runtime(
        socket,
        session,
        hub,
        authorizer,
        Arc::new(SystemClock),
        SocketTiming::protocol(),
    )
    .await;
}

pub async fn run_socket_with_runtime(
    socket: WebSocket,
    session: RealtimeSession,
    hub: LocalRealtimeHub,
    authorizer: Arc<dyn ConversationAuthorizer>,
    clock: Arc<dyn RealtimeClock>,
    timing: SocketTiming,
) {
    let connection = hub.register(session.user_id).await;
    let socket_id = connection.socket_id;
    let mut outbound = connection.outbound;
    let mut evictions = connection.evictions;
    let (mut sender, mut receiver) = socket.split();
    let expires_after = duration_until(session.access_token_expires_at, clock.now());
    let expiry = tokio::time::sleep(expires_after);
    let heartbeat = tokio::time::sleep(timing.heartbeat_timeout());
    tokio::pin!(expiry);
    tokio::pin!(heartbeat);

    loop {
        tokio::select! {
            biased;
            () = &mut expiry => {
                hub.cleanup(socket_id).await;
                close(&mut sender, CLOSE_REALTIME_AUTH_EXPIRED, "realtime_auth_expired").await;
                break;
            }
            () = &mut heartbeat => {
                hub.cleanup(socket_id).await;
                close(&mut sender, CLOSE_PROTOCOL_ERROR, "heartbeat_timeout").await;
                break;
            }
            eviction = evictions.recv() => {
                let Some(eviction) = eviction else {
                    break;
                };
                close(
                    &mut sender,
                    CLOSE_MEMBERSHIP_REQUIRED,
                    eviction.reason.as_str(),
                ).await;
                break;
            }
            message = receiver.next() => {
                match message {
                    Some(Ok(Message::Text(payload))) => {
                        heartbeat.as_mut().reset(Instant::now() + timing.heartbeat_timeout());
                        if !handle_client_frame(
                            payload.as_str(),
                            session.user_id,
                            socket_id,
                            &hub,
                            authorizer.as_ref(),
                            &mut sender,
                        ).await {
                            break;
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if sender.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Binary(_))) | Some(Err(_)) => {
                        hub.cleanup(socket_id).await;
                        close(&mut sender, CLOSE_PROTOCOL_ERROR, "protocol_error").await;
                        break;
                    }
                }
            }
            payload = outbound.recv() => {
                let Some(payload) = payload else {
                    break;
                };
                if sender.send(Message::Text(payload.into())).await.is_err() {
                    break;
                }
            }
        }
    }
    hub.cleanup(socket_id).await;
}

pub async fn run_unauthenticated_socket(mut socket: WebSocket) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code: CLOSE_REALTIME_AUTH_FAILED,
            reason: "realtime_auth_failed".into(),
        })))
        .await;
}

async fn handle_client_frame(
    payload: &str,
    user_id: Uuid,
    socket_id: crate::transport::realtime::SocketId,
    hub: &LocalRealtimeHub,
    authorizer: &dyn ConversationAuthorizer,
    sender: &mut SplitSink<WebSocket, Message>,
) -> bool {
    let frame = match serde_json::from_str::<ClientFrame>(payload) {
        Ok(frame) => frame,
        Err(_) => {
            hub.cleanup(socket_id).await;
            close(sender, CLOSE_PROTOCOL_ERROR, "protocol_error").await;
            return false;
        }
    };
    match frame {
        ClientFrame::Subscribe {
            request_id,
            conversation_id,
        } => {
            match authorizer.is_authorized(user_id, conversation_id).await {
                Ok(true) => {}
                Ok(false) => {
                    hub.cleanup(socket_id).await;
                    close(sender, CLOSE_MEMBERSHIP_REQUIRED, "membership_required").await;
                    return false;
                }
                Err(_) => {
                    hub.cleanup(socket_id).await;
                    close(sender, CLOSE_INTERNAL_ERROR, "internal_error").await;
                    return false;
                }
            }
            if !hub.subscribe(socket_id, conversation_id).await {
                close(sender, CLOSE_INTERNAL_ERROR, "internal_error").await;
                return false;
            }
            send_control(
                sender,
                &ServerControlFrame::Subscribed {
                    request_id,
                    conversation_id,
                },
            )
            .await
        }
        ClientFrame::Unsubscribe {
            request_id,
            conversation_id,
        } => {
            if !hub.unsubscribe(socket_id, conversation_id).await {
                close(sender, CLOSE_INTERNAL_ERROR, "internal_error").await;
                return false;
            }
            send_control(
                sender,
                &ServerControlFrame::Unsubscribed {
                    request_id,
                    conversation_id,
                },
            )
            .await
        }
        ClientFrame::Ping { nonce } => {
            if nonce.len() > MAX_NONCE_BYTES {
                hub.cleanup(socket_id).await;
                close(sender, CLOSE_PROTOCOL_ERROR, "protocol_error").await;
                return false;
            }
            send_control(sender, &ServerControlFrame::Pong { nonce }).await
        }
    }
}

async fn send_control(
    sender: &mut SplitSink<WebSocket, Message>,
    frame: &ServerControlFrame,
) -> bool {
    let Ok(payload) = serde_json::to_string(frame) else {
        close(sender, CLOSE_INTERNAL_ERROR, "internal_error").await;
        return false;
    };
    sender.send(Message::Text(payload.into())).await.is_ok()
}

async fn close(sender: &mut SplitSink<WebSocket, Message>, code: u16, reason: &'static str) {
    let _ = sender
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

fn duration_until(expires_at: time::OffsetDateTime, now: time::OffsetDateTime) -> Duration {
    let milliseconds = (expires_at - now).whole_milliseconds();
    u64::try_from(milliseconds)
        .ok()
        .filter(|milliseconds| *milliseconds > 0)
        .map(Duration::from_millis)
        .unwrap_or(Duration::ZERO)
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ClientFrame {
    Subscribe {
        request_id: Uuid,
        conversation_id: Uuid,
    },
    Unsubscribe {
        request_id: Uuid,
        conversation_id: Uuid,
    },
    Ping {
        nonce: String,
    },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerControlFrame {
    Subscribed {
        request_id: Uuid,
        conversation_id: Uuid,
    },
    Unsubscribed {
        request_id: Uuid,
        conversation_id: Uuid,
    },
    Pong {
        nonce: String,
    },
}
