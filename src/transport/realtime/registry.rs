use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

const SOCKET_BUFFER_CAPACITY: usize = 128;

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
}

#[derive(Clone, Default)]
pub struct LocalRealtimeHub {
    inner: Arc<Mutex<RegistryState>>,
}

impl LocalRealtimeHub {
    pub async fn register(&self, user_id: Uuid) -> SocketConnection {
        let socket_id = SocketId::new();
        let (sender, outbound) = mpsc::channel(SOCKET_BUFFER_CAPACITY);
        let mut state = self.inner.lock().await;
        state.sockets.insert(
            socket_id,
            SocketState {
                user_id,
                sender,
                conversations: HashSet::new(),
            },
        );
        state.users.entry(user_id).or_default().insert(socket_id);
        SocketConnection {
            socket_id,
            outbound,
        }
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
        let Some(socket) = state.sockets.remove(&socket_id) else {
            return false;
        };
        if let Some(sockets) = state.users.get_mut(&socket.user_id) {
            sockets.remove(&socket_id);
            if sockets.is_empty() {
                state.users.remove(&socket.user_id);
            }
        }
        for conversation_id in socket.conversations {
            if let Some(sockets) = state.conversations.get_mut(&conversation_id) {
                sockets.remove(&socket_id);
                if sockets.is_empty() {
                    state.conversations.remove(&conversation_id);
                }
            }
        }
        true
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

struct SocketState {
    user_id: Uuid,
    sender: mpsc::Sender<String>,
    conversations: HashSet<Uuid>,
}
