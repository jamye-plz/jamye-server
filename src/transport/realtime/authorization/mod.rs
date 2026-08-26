//! Final authoritative delivery authorization and local control application.

use std::{error::Error, fmt};

use uuid::Uuid;

use crate::{
    adapters::postgres::realtime_revocations::PostgresRealtimeRevocations,
    application::realtime::membership_revocation::RealtimeControlIntent,
    transport::realtime::{LocalRealtimeHub, RealtimeEvictionReason},
};

#[derive(Clone)]
pub struct AuthorizedRealtimeDelivery {
    hub: LocalRealtimeHub,
    authorization: PostgresRealtimeRevocations,
}

impl AuthorizedRealtimeDelivery {
    pub fn new(hub: LocalRealtimeHub, authorization: PostgresRealtimeRevocations) -> Self {
        Self { hub, authorization }
    }

    /// Performs one batched PostgreSQL check for all locally subscribed users.
    /// No registry sender is touched when PostgreSQL is unavailable or the group is deleted.
    pub async fn publish(
        &self,
        conversation_id: Uuid,
        payload: String,
    ) -> Result<usize, DeliveryAuthorizationError> {
        let candidates = self.hub.subscribed_users(conversation_id).await;
        if candidates.is_empty() {
            return Ok(0);
        }
        let authorized = self
            .authorization
            .authorized_users(conversation_id, &candidates)
            .await
            .map_err(|_| DeliveryAuthorizationError::Unavailable)?;
        Ok(self
            .hub
            .publish_authorized(conversation_id, payload, &authorized)
            .await)
    }
}

#[derive(Clone)]
pub struct RealtimeControlConsumer {
    hub: LocalRealtimeHub,
    topology: PostgresRealtimeRevocations,
}

impl RealtimeControlConsumer {
    pub fn new(hub: LocalRealtimeHub, topology: PostgresRealtimeRevocations) -> Self {
        Self { hub, topology }
    }

    pub async fn apply(
        &self,
        intent: &RealtimeControlIntent,
    ) -> Result<usize, RealtimeControlApplyError> {
        let conversation_ids = self
            .topology
            .conversation_ids(intent.group_id())
            .await
            .map_err(|_| RealtimeControlApplyError::Unavailable)?;
        let evicted = match intent {
            RealtimeControlIntent::MembershipRevoked { user_id, .. } => {
                self.hub
                    .evict_user_from_conversations(
                        *user_id,
                        &conversation_ids,
                        RealtimeEvictionReason::MembershipRevoked,
                    )
                    .await
            }
            RealtimeControlIntent::GroupDeleted { .. } => {
                self.hub
                    .evict_conversations(&conversation_ids, RealtimeEvictionReason::GroupDeleted)
                    .await
            }
        };
        Ok(evicted)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryAuthorizationError {
    Unavailable,
}

impl fmt::Display for DeliveryAuthorizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("realtime delivery authorization failed")
    }
}

impl Error for DeliveryAuthorizationError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RealtimeControlApplyError {
    Unavailable,
}

impl fmt::Display for RealtimeControlApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("realtime control application failed")
    }
}

impl Error for RealtimeControlApplyError {}
