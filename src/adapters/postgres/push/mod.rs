//! PostgreSQL Expo installation ownership adapter.

mod authorization;
mod delivery;
mod feedback;
mod mutation;
mod privacy;

use sqlx::PgPool;

use crate::ports::{
    push::{
        ClaimedPushDelivery, DeletePushInstallationCommand, FenceGroupPushCommand,
        FenceMembershipPushCommand, PushDeliveryClaim, PushDeliveryClaimRequest,
        PushDeliveryFailureCode, PushDeliveryFailureDisposition, PushDeliveryRepository,
        PushDeliveryRepositoryFuture, PushInstallationRecord, PushInvalidDestinationFuture,
        PushInvalidDestinationRepository, PushPreviewSource, PushPreviewSourceFuture,
        PushPrivacyFence, PushPrivacyFenceFuture, PushRepository, PushRepositoryError,
        PushRepositoryFuture, PushSendAuthorizationFuture, PushSendAuthorizationRepository,
        UpdatePushInstallationCommand, UpsertPushInstallationCommand,
        UpsertPushInstallationOutcome,
    },
    transactions::TransactionHandle,
};

use super::transactions::connection;

#[derive(Clone)]
pub struct PostgresPushRepository {
    pool: PgPool,
}

impl PostgresPushRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl PushRepository for PostgresPushRepository {
    fn upsert_installation<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a UpsertPushInstallationCommand,
    ) -> PushRepositoryFuture<'a, UpsertPushInstallationOutcome> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| PushRepositoryError::InvalidData)?;
            mutation::upsert_installation(connection, command).await
        })
    }

    fn update_installation<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a UpdatePushInstallationCommand,
    ) -> PushRepositoryFuture<'a, PushInstallationRecord> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| PushRepositoryError::InvalidData)?;
            mutation::update_installation(connection, command).await
        })
    }

    fn delete_installation<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a DeletePushInstallationCommand,
    ) -> PushRepositoryFuture<'a, ()> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| PushRepositoryError::InvalidData)?;
            mutation::delete_installation(connection, command).await
        })
    }
}

impl PushSendAuthorizationRepository for PostgresPushRepository {
    fn authorize_send<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        claim: &'a PushDeliveryClaim,
    ) -> PushSendAuthorizationFuture<'a> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| PushRepositoryError::InvalidData)?;
            authorization::authorize_send(connection, claim).await
        })
    }
}

impl PushPrivacyFence for PostgresPushRepository {
    fn fence_membership_revocation<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a FenceMembershipPushCommand,
    ) -> PushPrivacyFenceFuture<'a> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| PushRepositoryError::InvalidData)?;
            privacy::fence_membership_revocation(connection, command).await
        })
    }

    fn fence_group_deletion<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a FenceGroupPushCommand,
    ) -> PushPrivacyFenceFuture<'a> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| PushRepositoryError::InvalidData)?;
            privacy::fence_group_deletion(connection, command).await
        })
    }
}

impl PushDeliveryRepository for PostgresPushRepository {
    fn claim_deliveries(
        &self,
        request: PushDeliveryClaimRequest,
    ) -> PushDeliveryRepositoryFuture<'_, Vec<ClaimedPushDelivery>> {
        Box::pin(delivery::claim_deliveries(&self.pool, request))
    }

    fn mark_delivery_succeeded<'a>(
        &'a self,
        claim: &'a ClaimedPushDelivery,
    ) -> PushDeliveryRepositoryFuture<'a, bool> {
        Box::pin(delivery::mark_delivery_succeeded(&self.pool, claim))
    }

    fn record_delivery_failure<'a>(
        &'a self,
        claim: &'a ClaimedPushDelivery,
        code: PushDeliveryFailureCode,
        retry_delay: std::time::Duration,
        max_attempts: u32,
    ) -> PushDeliveryRepositoryFuture<'a, PushDeliveryFailureDisposition> {
        Box::pin(delivery::record_delivery_failure(
            &self.pool,
            claim,
            code,
            retry_delay,
            max_attempts,
        ))
    }
}

impl PushPreviewSource for PostgresPushRepository {
    fn load_message_body(&self, message_id: uuid::Uuid) -> PushPreviewSourceFuture<'_> {
        Box::pin(feedback::load_message_body(&self.pool, message_id))
    }
}

impl PushInvalidDestinationRepository for PostgresPushRepository {
    fn disable_invalid_destination<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        claim: &'a PushDeliveryClaim,
        destination: &'a crate::ports::push::ExpoPushDestination,
    ) -> PushInvalidDestinationFuture<'a> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| PushRepositoryError::InvalidData)?;
            feedback::disable_invalid_destination(connection, claim, destination).await
        })
    }
}

pub(super) fn database_error(operation: &'static str, _error: sqlx::Error) -> PushRepositoryError {
    tracing::warn!(
        dependency = "postgres",
        failure_kind = "push_installations",
        operation,
        "PostgreSQL push installation operation failed"
    );
    PushRepositoryError::Unavailable
}
