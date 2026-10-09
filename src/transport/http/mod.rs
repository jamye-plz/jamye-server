//! HTTP transport and static process composition.

pub mod account_deletion;
pub mod app_links;
pub mod auth;
pub mod avatar;
pub mod chatrooms;
pub mod composition;
#[cfg(feature = "dev-fixtures")]
pub mod dev_fixtures;
pub mod groups;
pub mod health;
pub mod legal;
pub mod media;
pub mod messaging;
pub mod moderation;
pub mod notifications;
pub mod push;
pub mod realtime;
pub mod topics;
pub mod users;
