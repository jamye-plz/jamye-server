//! Concrete in-process WebSocket registry and connection lifecycle.

pub mod authorization;
pub mod composition;
mod registry;
mod session;

pub use registry::{
    LocalRealtimeHub, RealtimeEviction, RealtimeEvictionReason, RegistryFull, SocketConnection,
    SocketId,
};
pub(crate) use session::run_shutdown_socket;
pub use session::{
    CLOSE_GOING_AWAY, CLOSE_INTERNAL_ERROR, CLOSE_MEMBERSHIP_REQUIRED, CLOSE_MESSAGE_TOO_BIG,
    CLOSE_PROTOCOL_ERROR, CLOSE_REALTIME_AUTH_EXPIRED, CLOSE_REALTIME_AUTH_FAILED,
    CLOSE_TRY_AGAIN_LATER, SocketTiming, run_socket, run_socket_with_runtime,
    run_unauthenticated_socket,
};
