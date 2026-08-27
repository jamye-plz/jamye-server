use std::time::Duration;

use jamye_server::{
    adapters::postgres::{push::PostgresPushRepository, transactions::SqlxTransactionManager},
    application::realtime::membership_revocation::{
        MembershipRevocationError, MembershipRevocationService,
    },
    ports::{
        push::{
            FenceGroupPushCommand, FenceMembershipPushCommand, PushPrivacyFence,
            PushPrivacyFenceFuture, PushRepositoryError, PushSendAuthorizationRepository,
        },
        transactions::TransactionManager,
    },
};

use crate::{
    TestResult,
    postgres_support::TestDatabase,
    send_authorization::helpers::{SendTopology, committed_authorize},
};

#[path = "privacy_mutations/helpers.rs"]
mod helpers;

use helpers::{
    assert_occurrence_fenced, membership_revocations, membership_revocations_with_privacy,
};

#[derive(Clone, Copy)]
enum PrivacyMutation {
    MembershipRevocation,
    GroupDeletion,
}

#[tokio::test]
async fn privacy_fence_failure_rolls_back_group_state_and_control_intent() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let topology = SendTopology::new(&pool).await?;
    let service = membership_revocations_with_privacy(pool.clone(), UnavailablePrivacyFence)?;

    assert_eq!(
        service
            .remove_member(topology.owner_id, topology.group_id, topology.recipient_id)
            .await,
        Err(MembershipRevocationError::Push(
            PushRepositoryError::Unavailable
        ))
    );
    let membership_exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM memberships WHERE group_id = $1 AND user_id = $2)",
    )
    .bind(topology.group_id)
    .bind(topology.recipient_id)
    .fetch_one(&pool)
    .await?;
    assert!(membership_exists);

    assert_eq!(
        service
            .delete_group(topology.owner_id, topology.group_id)
            .await,
        Err(MembershipRevocationError::Push(
            PushRepositoryError::Unavailable
        ))
    );
    let group_is_live =
        sqlx::query_scalar::<_, bool>("SELECT deleted_at IS NULL FROM groups WHERE id = $1")
            .bind(topology.group_id)
            .fetch_one(&pool)
            .await?;
    assert!(group_is_live);
    let control_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM outbox_events WHERE intent_type = 'control'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(control_count, 0);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn committed_membership_and_group_mutations_terminalize_old_push_attempts() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let service = membership_revocations(pool.clone())?;
    let repository = PostgresPushRepository::new(pool.clone());
    let transactions = SqlxTransactionManager::new(pool.clone());

    for mutation in [
        PrivacyMutation::MembershipRevocation,
        PrivacyMutation::GroupDeletion,
    ] {
        let topology = SendTopology::new(&pool).await?;
        apply_mutation(&service, &topology, mutation).await?;
        assert_occurrence_fenced(&pool, topology.occurrence_id).await?;
        assert!(
            committed_authorize(&repository, &transactions, &topology.claim)
                .await?
                .is_none()
        );
    }

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn authorization_first_blocks_both_group_privacy_mutations_then_fences_retries() -> TestResult
{
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let service = membership_revocations(pool.clone())?;
    let repository = PostgresPushRepository::new(pool.clone());
    let transactions = SqlxTransactionManager::new(pool.clone());

    for mutation in [
        PrivacyMutation::MembershipRevocation,
        PrivacyMutation::GroupDeletion,
    ] {
        let topology = SendTopology::new(&pool).await?;
        assert_authorization_first(&repository, &transactions, &service, &topology, mutation)
            .await?;
        assert_occurrence_fenced(&pool, topology.occurrence_id).await?;
        assert!(
            committed_authorize(&repository, &transactions, &topology.claim)
                .await?
                .is_none()
        );
    }

    pool.close().await;
    database.dispose().await
}

async fn assert_authorization_first(
    repository: &PostgresPushRepository,
    transactions: &SqlxTransactionManager,
    service: &MembershipRevocationService,
    topology: &SendTopology,
    mutation: PrivacyMutation,
) -> TestResult {
    let mut authorization_transaction = transactions.begin().await?;
    let authorized = repository
        .authorize_send(authorization_transaction.as_mut(), &topology.claim)
        .await?
        .ok_or("live occurrence was not authorized before the privacy mutation")?;
    let mut pending_mutation = Box::pin(apply_mutation(service, topology, mutation));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut pending_mutation)
            .await
            .is_err(),
        "privacy mutation bypassed the uncommitted send-authorization lock"
    );
    transactions.commit(authorization_transaction).await?;
    assert_eq!(authorized.destination.token(), topology.expo_token.as_str());
    pending_mutation.await
}

async fn apply_mutation(
    service: &MembershipRevocationService,
    topology: &SendTopology,
    mutation: PrivacyMutation,
) -> TestResult {
    match mutation {
        PrivacyMutation::MembershipRevocation => {
            service
                .remove_member(topology.owner_id, topology.group_id, topology.recipient_id)
                .await?;
        }
        PrivacyMutation::GroupDeletion => {
            service
                .delete_group(topology.owner_id, topology.group_id)
                .await?;
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct UnavailablePrivacyFence;

impl PushPrivacyFence for UnavailablePrivacyFence {
    fn fence_membership_revocation<'a>(
        &'a self,
        _transaction: &'a mut dyn jamye_server::ports::transactions::TransactionHandle,
        _command: &'a FenceMembershipPushCommand,
    ) -> PushPrivacyFenceFuture<'a> {
        Box::pin(async { Err(PushRepositoryError::Unavailable) })
    }

    fn fence_group_deletion<'a>(
        &'a self,
        _transaction: &'a mut dyn jamye_server::ports::transactions::TransactionHandle,
        _command: &'a FenceGroupPushCommand,
    ) -> PushPrivacyFenceFuture<'a> {
        Box::pin(async { Err(PushRepositoryError::Unavailable) })
    }
}
