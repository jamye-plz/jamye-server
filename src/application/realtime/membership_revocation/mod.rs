//! Atomic membership mutation plus durable realtime-control intent orchestration.

use std::{error::Error, fmt, future::Future, pin::Pin, sync::Arc};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    application::groups::{GroupsError, GroupsService},
    ports::{
        push::{
            FenceGroupPushCommand, FenceMembershipPushCommand, PushPrivacyFence,
            PushRepositoryError,
        },
        transactions::{BoxTransactionHandle, TransactionHandle, TransactionManager},
    },
};

pub const REALTIME_CONTROL_VERSION: i16 = 1;

pub type ControlIntentFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), ControlIntentError>> + Send + 'a>>;

/// Task-local persistence collaborator. It deliberately does not extend the frozen
/// task-4b realtime port or task-6 groups port.
pub trait ControlIntentAppender: Send + Sync {
    fn append<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        intent: &'a RealtimeControlIntent,
    ) -> ControlIntentFuture<'a>;
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum RealtimeControlIntent {
    #[serde(rename = "membership.revoked")]
    MembershipRevoked {
        version: i16,
        control_id: Uuid,
        group_id: Uuid,
        user_id: Uuid,
    },
    #[serde(rename = "group.deleted")]
    GroupDeleted {
        version: i16,
        control_id: Uuid,
        group_id: Uuid,
    },
}

impl RealtimeControlIntent {
    pub fn membership_revoked(group_id: Uuid, user_id: Uuid) -> Self {
        Self::MembershipRevoked {
            version: REALTIME_CONTROL_VERSION,
            control_id: Uuid::new_v4(),
            group_id,
            user_id,
        }
    }

    pub fn group_deleted(group_id: Uuid) -> Self {
        Self::GroupDeleted {
            version: REALTIME_CONTROL_VERSION,
            control_id: Uuid::new_v4(),
            group_id,
        }
    }

    pub fn control_id(&self) -> Uuid {
        match self {
            Self::MembershipRevoked { control_id, .. } | Self::GroupDeleted { control_id, .. } => {
                *control_id
            }
        }
    }

    pub fn group_id(&self) -> Uuid {
        match self {
            Self::MembershipRevoked { group_id, .. } | Self::GroupDeleted { group_id, .. } => {
                *group_id
            }
        }
    }

    pub fn version(&self) -> i16 {
        match self {
            Self::MembershipRevoked { version, .. } | Self::GroupDeleted { version, .. } => {
                *version
            }
        }
    }

    pub fn event_type(&self) -> &'static str {
        match self {
            Self::MembershipRevoked { .. } => "membership.revoked",
            Self::GroupDeleted { .. } => "group.deleted",
        }
    }

    pub fn aggregate_type(&self) -> &'static str {
        match self {
            Self::MembershipRevoked { .. } => "membership",
            Self::GroupDeleted { .. } => "group",
        }
    }

    pub fn aggregate_id(&self) -> Uuid {
        match self {
            Self::MembershipRevoked { user_id, .. } => *user_id,
            Self::GroupDeleted { group_id, .. } => *group_id,
        }
    }
}

#[derive(Clone)]
pub struct MembershipRevocationService {
    groups: Arc<GroupsService>,
    transactions: Arc<dyn TransactionManager>,
    intents: Arc<dyn ControlIntentAppender>,
    push_privacy: Arc<dyn PushPrivacyFence>,
}

impl MembershipRevocationService {
    pub fn new(
        groups: Arc<GroupsService>,
        transactions: Arc<dyn TransactionManager>,
        intents: Arc<dyn ControlIntentAppender>,
        push_privacy: Arc<dyn PushPrivacyFence>,
    ) -> Self {
        Self {
            groups,
            transactions,
            intents,
            push_privacy,
        }
    }

    pub async fn remove_member(
        &self,
        actor_id: Uuid,
        group_id: Uuid,
        target_user_id: Uuid,
    ) -> Result<RealtimeControlIntent, MembershipRevocationError> {
        let mut transaction = self.begin().await?;
        let mutation = self
            .groups
            .remove_member_in_transaction(transaction.as_mut(), actor_id, group_id, target_user_id)
            .await
            .map_err(MembershipRevocationError::Group);
        if let Err(error) = mutation {
            return self.finish(transaction, Err(error)).await;
        }
        let privacy = self
            .push_privacy
            .fence_membership_revocation(
                transaction.as_mut(),
                &FenceMembershipPushCommand {
                    group_id,
                    user_id: target_user_id,
                },
            )
            .await
            .map_err(MembershipRevocationError::Push);
        if let Err(error) = privacy {
            return self.finish(transaction, Err(error)).await;
        }
        let intent = RealtimeControlIntent::membership_revoked(group_id, target_user_id);
        let result = self
            .intents
            .append(transaction.as_mut(), &intent)
            .await
            .map(|()| intent)
            .map_err(MembershipRevocationError::ControlIntent);
        self.finish(transaction, result).await
    }

    pub async fn delete_group(
        &self,
        actor_id: Uuid,
        group_id: Uuid,
    ) -> Result<RealtimeControlIntent, MembershipRevocationError> {
        let mut transaction = self.begin().await?;
        let mutation = self
            .groups
            .delete_group_in_transaction(transaction.as_mut(), actor_id, group_id)
            .await
            .map_err(MembershipRevocationError::Group);
        if let Err(error) = mutation {
            return self.finish(transaction, Err(error)).await;
        }
        let privacy = self
            .push_privacy
            .fence_group_deletion(transaction.as_mut(), &FenceGroupPushCommand { group_id })
            .await
            .map_err(MembershipRevocationError::Push);
        if let Err(error) = privacy {
            return self.finish(transaction, Err(error)).await;
        }
        let intent = RealtimeControlIntent::group_deleted(group_id);
        let result = self
            .intents
            .append(transaction.as_mut(), &intent)
            .await
            .map(|()| intent)
            .map_err(MembershipRevocationError::ControlIntent);
        self.finish(transaction, result).await
    }

    async fn begin(&self) -> Result<BoxTransactionHandle, MembershipRevocationError> {
        self.transactions
            .begin()
            .await
            .map_err(|_| MembershipRevocationError::Transaction)
    }

    async fn finish<T>(
        &self,
        transaction: BoxTransactionHandle,
        result: Result<T, MembershipRevocationError>,
    ) -> Result<T, MembershipRevocationError> {
        match result {
            Ok(value) => {
                self.transactions
                    .commit(transaction)
                    .await
                    .map_err(|_| MembershipRevocationError::Transaction)?;
                Ok(value)
            }
            Err(error) => {
                self.transactions
                    .rollback(transaction)
                    .await
                    .map_err(|_| MembershipRevocationError::Transaction)?;
                Err(error)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlIntentError {
    Unavailable,
    InvalidData,
}

impl fmt::Display for ControlIntentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("realtime control intent operation failed")
    }
}

impl Error for ControlIntentError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MembershipRevocationError {
    Group(GroupsError),
    Push(PushRepositoryError),
    ControlIntent(ControlIntentError),
    Transaction,
}

impl fmt::Display for MembershipRevocationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("membership revocation operation failed")
    }
}

impl Error for MembershipRevocationError {}
