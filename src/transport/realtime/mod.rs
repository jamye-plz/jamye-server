//! Concrete in-process WebSocket registry and connection lifecycle.

pub mod authorization;
pub mod composition;
mod registry;
mod session;

pub use registry::{
    LocalRealtimeHub, RealtimeEviction, RealtimeEvictionReason, SocketConnection, SocketId,
};
pub use session::{
    CLOSE_INTERNAL_ERROR, CLOSE_MEMBERSHIP_REQUIRED, CLOSE_PROTOCOL_ERROR,
    CLOSE_REALTIME_AUTH_EXPIRED, CLOSE_REALTIME_AUTH_FAILED, SocketTiming, run_socket,
    run_socket_with_runtime, run_unauthenticated_socket,
};
