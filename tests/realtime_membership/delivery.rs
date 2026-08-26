use jamye_server::transport::realtime::{
    LocalRealtimeHub,
    authorization::{AuthorizedRealtimeDelivery, DeliveryAuthorizationError},
};
use tokio::sync::mpsc::error::TryRecvError;

use crate::{
    TestResult,
    helpers::{create_group, harness, insert_member, insert_user},
    postgres_support::TestDatabase,
};

#[tokio::test]
async fn dropped_control_signal_cannot_bypass_final_authoritative_delivery_check() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "전달 소유자").await?;
    let member_id = insert_user(&pool, "전달 멤버").await?;
    let group = create_group(&fixture, owner_id, "전달 그룹").await?;
    insert_member(&pool, group.id, member_id).await?;

    let hub = LocalRealtimeHub::default();
    let mut connection = hub.register(member_id).await;
    assert!(hub.subscribe(connection.socket_id, group.main_chatroom_id).await);
    let delivery = AuthorizedRealtimeDelivery::new(hub.clone(), fixture.store.clone());

    assert_eq!(
        delivery
            .publish(group.main_chatroom_id, "before-commit".to_owned())
            .await?,
        1
    );
    assert_eq!(connection.outbound.recv().await.as_deref(), Some("before-commit"));

    fixture
        .revocations
        .remove_member(owner_id, group.id, member_id)
        .await?;
    // Intentionally do not consume or apply the Redis control on this node.
    assert_eq!(hub.registry_counts().await, (1, 1, 1));
    assert_eq!(
        delivery
            .publish(group.main_chatroom_id, "after-commit".to_owned())
            .await?,
        0
    );
    assert_eq!(connection.outbound.try_recv(), Err(TryRecvError::Empty));

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn deleted_group_and_database_uncertainty_both_fail_closed_without_delivery() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "폐쇄 소유자").await?;
    let group = create_group(&fixture, owner_id, "폐쇄 그룹").await?;
    let hub = LocalRealtimeHub::default();
    let mut connection = hub.register(owner_id).await;
    assert!(hub.subscribe(connection.socket_id, group.main_chatroom_id).await);
    let delivery = AuthorizedRealtimeDelivery::new(hub, fixture.store.clone());

    fixture
        .revocations
        .delete_group(owner_id, group.id)
        .await?;
    assert_eq!(
        delivery
            .publish(group.main_chatroom_id, "deleted".to_owned())
            .await?,
        0
    );
    assert_eq!(connection.outbound.try_recv(), Err(TryRecvError::Empty));

    pool.close().await;
    assert_eq!(
        delivery
            .publish(group.main_chatroom_id, "database-down".to_owned())
            .await,
        Err(DeliveryAuthorizationError::Unavailable)
    );
    assert_eq!(connection.outbound.try_recv(), Err(TryRecvError::Empty));
    database.dispose().await
}

#[tokio::test]
async fn every_delivery_check_started_after_revocation_commit_excludes_the_former_member(
) -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "경합 소유자").await?;
    let member_id = insert_user(&pool, "경합 멤버").await?;
    let group = create_group(&fixture, owner_id, "경합 그룹").await?;
    insert_member(&pool, group.id, member_id).await?;
    let hub = LocalRealtimeHub::default();
    let mut connection = hub.register(member_id).await;
    assert!(hub.subscribe(connection.socket_id, group.main_chatroom_id).await);
    let delivery = AuthorizedRealtimeDelivery::new(hub.clone(), fixture.store.clone());

    fixture
        .revocations
        .remove_member(owner_id, group.id, member_id)
        .await?;
    let delivery_after_commit = tokio::spawn(async move {
        delivery
            .publish(group.main_chatroom_id, "post-commit-race".to_owned())
            .await
    });
    assert_eq!(delivery_after_commit.await??, 0);
    assert_eq!(hub.registry_counts().await, (1, 1, 1));
    assert_eq!(connection.outbound.try_recv(), Err(TryRecvError::Empty));

    pool.close().await;
    database.dispose().await
}
