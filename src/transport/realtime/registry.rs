use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
    time::Duration,
};

use tokio::sync::{Mutex, mpsc, watch};
use uuid::Uuid;

mod revocation;

pub use revocation::{RealtimeEviction, RealtimeEvictionReason};

const SOCKET_BUFFER_CAPACITY: usize = 128;
const MAX_SOCKETS_PER_USER: usize = 8;

#[derive(Debug)]
pub struct RegistryFull;

impl fmt::Display for RegistryFull {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("per-user socket limit reached")
    }
}

impl std::error::Error for RegistryFull {}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SocketId(Uuid);

impl SocketId {
    fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

pub struct SocketConnection {
    pub socket_id: SocketId,
    pub outbound: mpsc::Receiver<String>,
    pub evictions: mpsc::Receiver<RealtimeEviction>,
    pub(super) session: SocketSession,
}

#[derive(Clone)]
pub struct LocalRealtimeHub {
    inner: Arc<Mutex<RegistryState>>,
    shutdown: watch::Sender<bool>,
    active_sessions: watch::Sender<usize>,
}

impl Default for LocalRealtimeHub {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RegistryState::default())),
            shutdown: watch::channel(false).0,
            active_sessions: watch::channel(0).0,
        }
    }
}

pub(super) enum RegisterError {
    Full,
    ShuttingDown,
}

pub(super) struct SocketSession {
    active_sessions: watch::Sender<usize>,
}

impl Drop for SocketSession {
    fn drop(&mut self) {
        self.active_sessions.send_modify(|count| *count -= 1);
    }
}

impl LocalRealtimeHub {
    /// Test-facing registration; after `shutdown` it also returns `RegistryFull`.
    pub async fn register(&self, user_id: Uuid) -> Result<SocketConnection, RegistryFull> {
        self.register_socket(user_id)
            .await
            .map_err(|_| RegistryFull)
    }

    pub(super) async fn register_socket(
        &self,
        user_id: Uuid,
    ) -> Result<SocketConnection, RegisterError> {
        let mut state = self.inner.lock().await;
        if *self.shutdown.borrow() {
            return Err(RegisterError::ShuttingDown);
        }
        if state
            .users
            .get(&user_id)
            .is_some_and(|sockets| sockets.len() >= MAX_SOCKETS_PER_USER)
        {
            return Err(RegisterError::Full);
        }
        let socket_id = SocketId::new();
        let (sender, outbound) = mpsc::channel(SOCKET_BUFFER_CAPACITY);
        let (eviction_sender, evictions) = mpsc::channel(1);
        state.sockets.insert(
            socket_id,
            SocketState {
                user_id,
                sender,
                eviction_sender,
                conversations: HashSet::new(),
            },
        );
        state.users.entry(user_id).or_default().insert(socket_id);
        self.active_sessions.send_modify(|count| *count += 1);
        Ok(SocketConnection {
            socket_id,
            outbound,
            evictions,
            session: SocketSession {
                active_sessions: self.active_sessions.clone(),
            },
        })
    }

    pub async fn shutdown(&self) {
        // Serialize shutdown with admission so no session can appear after draining starts.
        let _state = self.inner.lock().await;
        self.shutdown.send_replace(true);
    }

    pub(crate) fn is_shutting_down(&self) -> bool {
        *self.shutdown.borrow()
    }

    pub(super) fn shutdown_signal(&self) -> watch::Receiver<bool> {
        self.shutdown.subscribe()
    }

    /// After shutdown, wait for sessions to finish their final close attempt.
    pub async fn wait_for_drain(&self, limit: Duration) -> bool {
        let mut active_sessions = self.active_sessions.subscribe();
        matches!(
            tokio::time::timeout(limit, async {
                loop {
                    let drained = *active_sessions.borrow_and_update() == 0;
                    if drained {
                        return true;
                    }
                    if active_sessions.changed().await.is_err() {
                        return false;
                    }
                }
            })
            .await,
            Ok(true)
        )
    }

    pub async fn subscribe(&self, socket_id: SocketId, conversation_id: Uuid) -> bool {
        let mut state = self.inner.lock().await;
        let Some(socket) = state.sockets.get_mut(&socket_id) else {
            return false;
        };
        socket.conversations.insert(conversation_id);
        state
            .conversations
            .entry(conversation_id)
            .or_default()
            .insert(socket_id);
        true
    }

    pub async fn unsubscribe(&self, socket_id: SocketId, conversation_id: Uuid) -> bool {
        let mut state = self.inner.lock().await;
        let Some(socket) = state.sockets.get_mut(&socket_id) else {
            return false;
        };
        socket.conversations.remove(&conversation_id);
        if let Some(sockets) = state.conversations.get_mut(&conversation_id) {
            sockets.remove(&socket_id);
            if sockets.is_empty() {
                state.conversations.remove(&conversation_id);
            }
        }
        true
    }

    pub async fn cleanup(&self, socket_id: SocketId) -> bool {
        let mut state = self.inner.lock().await;
        state.remove_socket(socket_id).is_some()
    }

    pub async fn publish(&self, conversation_id: Uuid, payload: String) -> usize {
        let recipients = {
            let state = self.inner.lock().await;
            state
                .conversations
                .get(&conversation_id)
                .into_iter()
                .flatten()
                .filter_map(|socket_id| {
                    state
                        .sockets
                        .get(socket_id)
                        .map(|socket| (*socket_id, socket.sender.clone()))
                })
                .collect::<Vec<_>>()
        };
        let mut delivered = 0;
        let mut disconnected = Vec::new();
        for (socket_id, sender) in recipients {
            match sender.try_send(payload.clone()) {
                Ok(()) => delivered += 1,
                Err(mpsc::error::TrySendError::Closed(_)) => disconnected.push(socket_id),
                Err(mpsc::error::TrySendError::Full(_)) => {}
            }
        }
        for socket_id in disconnected {
            self.cleanup(socket_id).await;
        }
        delivered
    }

    pub async fn subscribed_users(&self, conversation_id: Uuid) -> Vec<Uuid> {
        let state = self.inner.lock().await;
        let mut users = state
            .conversations
            .get(&conversation_id)
            .into_iter()
            .flatten()
            .filter_map(|socket_id| state.sockets.get(socket_id).map(|socket| socket.user_id))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        users.sort_unstable();
        users
    }

    pub async fn publish_authorized(
        &self,
        conversation_id: Uuid,
        payload: String,
        authorized_users: &HashSet<Uuid>,
    ) -> usize {
        let recipients = {
            let state = self.inner.lock().await;
            state
                .conversations
                .get(&conversation_id)
                .into_iter()
                .flatten()
                .filter_map(|socket_id| {
                    state.sockets.get(socket_id).and_then(|socket| {
                        authorized_users
                            .contains(&socket.user_id)
                            .then(|| (*socket_id, socket.sender.clone()))
                    })
                })
                .collect::<Vec<_>>()
        };
        let mut delivered = 0;
        let mut disconnected = Vec::new();
        for (socket_id, sender) in recipients {
            match sender.try_send(payload.clone()) {
                Ok(()) => delivered += 1,
                Err(mpsc::error::TrySendError::Closed(_)) => disconnected.push(socket_id),
                Err(mpsc::error::TrySendError::Full(_)) => {}
            }
        }
        for socket_id in disconnected {
            self.cleanup(socket_id).await;
        }
        delivered
    }

    pub async fn evict_user_from_conversations(
        &self,
        user_id: Uuid,
        conversation_ids: &[Uuid],
        reason: RealtimeEvictionReason,
    ) -> usize {
        let conversations = conversation_ids.iter().copied().collect::<HashSet<_>>();
        let senders = {
            let mut state = self.inner.lock().await;
            let socket_ids = state
                .users
                .get(&user_id)
                .into_iter()
                .flatten()
                .copied()
                .filter(|socket_id| {
                    state.sockets.get(socket_id).is_some_and(|socket| {
                        socket
                            .conversations
                            .iter()
                            .any(|conversation_id| conversations.contains(conversation_id))
                    })
                })
                .collect::<Vec<_>>();
            socket_ids
                .into_iter()
                .filter_map(|socket_id| {
                    state
                        .remove_socket(socket_id)
                        .map(|socket| socket.eviction_sender)
                })
                .collect::<Vec<_>>()
        };
        send_evictions(senders, reason)
    }

    pub async fn evict_conversations(
        &self,
        conversation_ids: &[Uuid],
        reason: RealtimeEvictionReason,
    ) -> usize {
        let senders = {
            let mut state = self.inner.lock().await;
            let socket_ids = conversation_ids
                .iter()
                .filter_map(|conversation_id| state.conversations.get(conversation_id))
                .flatten()
                .copied()
                .collect::<HashSet<_>>();
            socket_ids
                .into_iter()
                .filter_map(|socket_id| {
                    state
                        .remove_socket(socket_id)
                        .map(|socket| socket.eviction_sender)
                })
                .collect::<Vec<_>>()
        };
        send_evictions(senders, reason)
    }

    pub async fn registry_counts(&self) -> (usize, usize, usize) {
        let state = self.inner.lock().await;
        (
            state.sockets.len(),
            state.users.values().map(HashSet::len).sum(),
            state.conversations.values().map(HashSet::len).sum(),
        )
    }
}

#[derive(Default)]
struct RegistryState {
    sockets: HashMap<SocketId, SocketState>,
    users: HashMap<Uuid, HashSet<SocketId>>,
    conversations: HashMap<Uuid, HashSet<SocketId>>,
}

impl RegistryState {
    fn remove_socket(&mut self, socket_id: SocketId) -> Option<SocketState> {
        let socket = self.sockets.remove(&socket_id)?;
        if let Some(sockets) = self.users.get_mut(&socket.user_id) {
            sockets.remove(&socket_id);
            if sockets.is_empty() {
                self.users.remove(&socket.user_id);
            }
        }
        for conversation_id in &socket.conversations {
            if let Some(sockets) = self.conversations.get_mut(conversation_id) {
                sockets.remove(&socket_id);
                if sockets.is_empty() {
                    self.conversations.remove(conversation_id);
                }
            }
        }
        Some(socket)
    }
}

struct SocketState {
    user_id: Uuid,
    sender: mpsc::Sender<String>,
    eviction_sender: mpsc::Sender<RealtimeEviction>,
    conversations: HashSet<Uuid>,
}

fn send_evictions(
    senders: Vec<mpsc::Sender<RealtimeEviction>>,
    reason: RealtimeEvictionReason,
) -> usize {
    let mut sent = 0;
    for sender in senders {
        if sender.try_send(RealtimeEviction::new(reason)).is_ok() {
            sent += 1;
        }
    }
    sent
}
