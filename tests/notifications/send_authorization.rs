use std::time::Duration;

use jamye_server::{
    adapters::postgres::{push::PostgresPushRepository, transactions::SqlxTransactionManager},
    ports::{
        push::{
            DeletePushInstallationCommand, NotificationType, PushEnvironment,
            PushSendAuthorizationRepository, UpdatePushInstallationCommand,
        },
        transactions::TransactionManager,
    },
};

use crate::{TestResult, postgres_support::TestDatabase};

#[path = "send_authorization/helpers.rs"]
pub(crate) mod helpers;

use helpers::{
    MESSAGE_BODY, SendTopology, committed_authorize, committed_delete, committed_update,
};

#[tokio::test]
async fn send_authorization_revalidates_route_epoch_preview_and_claim_generation() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let topology = SendTopology::new(&pool).await?;
    let repository = PostgresPushRepository::new(pool.clone());
    let transactions = SqlxTransactionManager::new(pool.clone());

    let authorized = committed_authorize(&repository, &transactions, &topology.claim)
        .await?
        .ok_or("live push claim was not authorized")?;
    assert_eq!(authorized.occurrence_id, topology.occurrence_id);
    assert_eq!(
        authorized.route.notification_type,
        NotificationType::ChatUnread
    );
    assert_eq!(authorized.route.notification_id, topology.notification_id);
    assert_eq!(authorized.route.conversation_id, topology.conversation_id);
    assert_eq!(authorized.route.message_id, Some(topology.message_id));
    assert_eq!(
        authorized.destination.environment(),
        PushEnvironment::Development
    );
    assert_eq!(authorized.destination.token(), topology.expo_token.as_str());
    assert_eq!(authorized.preview_message_id, Some(topology.message_id));
    let debug = format!("{authorized:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains(&topology.expo_token));
    assert!(!debug.contains(MESSAGE_BODY));

    sqlx::query("UPDATE push_installations SET message_preview_enabled = false WHERE id = $1")
        .bind(topology.installation_id)
        .execute(&pool)
        .await?;
    let generic = committed_authorize(&repository, &transactions, &topology.claim)
        .await?
        .ok_or("preview-disabled installation lost identifier-only delivery")?;
    assert_eq!(generic.route, authorized.route);
    assert_eq!(generic.destination.token(), topology.expo_token.as_str());
    assert_eq!(generic.preview_message_id, None);

    let mut stale_generation = topology.claim.clone();
    stale_generation.claim_generation += 1;
    assert!(
        committed_authorize(&repository, &transactions, &stale_generation)
            .await?
            .is_none()
    );
    let mut stale_owner = topology.claim.clone();
    stale_owner.claim_owner = "other-worker".to_owned();
    assert!(
        committed_authorize(&repository, &transactions, &stale_owner)
            .await?
            .is_none()
    );

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn committed_privacy_mutations_remove_all_old_attempt_material() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let repository = PostgresPushRepository::new(pool.clone());
    let transactions = SqlxTransactionManager::new(pool.clone());

    let removed_member = SendTopology::new(&pool).await?;
    sqlx::query("DELETE FROM memberships WHERE group_id = $1 AND user_id = $2")
        .bind(removed_member.group_id)
        .bind(removed_member.recipient_id)
        .execute(&pool)
        .await?;
    assert!(
        committed_authorize(&repository, &transactions, &removed_member.claim)
            .await?
            .is_none()
    );

    let deleted_group = SendTopology::new(&pool).await?;
    sqlx::query("UPDATE groups SET deleted_at = clock_timestamp() WHERE id = $1")
        .bind(deleted_group.group_id)
        .execute(&pool)
        .await?;
    assert!(
        committed_authorize(&repository, &transactions, &deleted_group.claim)
            .await?
            .is_none()
    );

    let rebound = SendTopology::new(&pool).await?;
    let rotated = committed_update(
        &repository,
        &transactions,
        UpdatePushInstallationCommand {
            user_id: rebound.recipient_id,
            installation_id: rebound.public_installation_id.clone(),
            token: "ExponentPushToken[rotated-before-authorization]".to_owned(),
            message_preview_enabled: None,
        },
    )
    .await?;
    assert_eq!(rotated.owner_epoch, 2);
    assert!(
        committed_authorize(&repository, &transactions, &rebound.claim)
            .await?
            .is_none()
    );

    let deleted_installation = SendTopology::new(&pool).await?;
    committed_delete(
        &repository,
        &transactions,
        DeletePushInstallationCommand {
            user_id: deleted_installation.recipient_id,
            installation_id: deleted_installation.public_installation_id.clone(),
        },
    )
    .await?;
    assert!(
        committed_authorize(&repository, &transactions, &deleted_installation.claim)
            .await?
            .is_none()
    );

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn authorization_first_fences_rebind_then_every_later_attempt_is_denied() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let topology = SendTopology::new(&pool).await?;
    let repository = PostgresPushRepository::new(pool.clone());
    let transactions = SqlxTransactionManager::new(pool.clone());

    let mut authorization_transaction = transactions.begin().await?;
    let authorized = repository
        .authorize_send(authorization_transaction.as_mut(), &topology.claim)
        .await?
        .ok_or("live claim was not authorized before the rebind")?;

    let rebind_repository = repository.clone();
    let rebind_transactions = SqlxTransactionManager::new(pool.clone());
    let rebind_user_id = topology.recipient_id;
    let rebind_installation_id = topology.public_installation_id.clone();
    let mut rebind = Box::pin(async move {
        committed_update(
            &rebind_repository,
            &rebind_transactions,
            UpdatePushInstallationCommand {
                user_id: rebind_user_id,
                installation_id: rebind_installation_id,
                token: "ExponentPushToken[rotated-after-authorization]".to_owned(),
                message_preview_enabled: None,
            },
        )
        .await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut rebind)
            .await
            .is_err(),
        "installation rebind bypassed the uncommitted send-authorization lock"
    );
    transactions.commit(authorization_transaction).await?;
    assert_eq!(authorized.destination.token(), topology.expo_token.as_str());
    assert_eq!(rebind.await?.owner_epoch, 2);
    assert!(
        committed_authorize(&repository, &transactions, &topology.claim)
            .await?
            .is_none()
    );

    pool.close().await;
    database.dispose().await
}
