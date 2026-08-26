//! HTTP transport and static process composition.

pub mod auth;
pub mod chatrooms;
pub mod composition;
#[cfg(feature = "dev-fixtures")]
pub mod dev_fixtures;
pub mod groups;
pub mod health;
pub mod messaging;
pub mod realtime;
pub mod users;
