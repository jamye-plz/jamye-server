use jamye_server::{
    adapters::postgres::{push::PostgresPushRepository, transactions::SqlxTransactionManager},
    ports::{
        push::{
            ExpoPushDestination, PushDeliveryClaim, PushEnvironment,
            PushInvalidDestinationRepository, PushPreviewSource, UpdatePushInstallationCommand,
        },
        transactions::TransactionManager,
    },
};
use uuid::Uuid;

use crate::{
    TestResult,
    postgres_support::TestDatabase,
    send_authorization::helpers::{MESSAGE_BODY, SendTopology, committed_update},
};

#[tokio::test]
async fn postgres_preview_source_reads_only_the_canonical_nullable_message_body() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let topology = SendTopology::new(&pool).await?;
    let repository = PostgresPushRepository::new(pool.clone());

    assert_eq!(
        repository.load_message_body(topology.message_id).await?,
        Some(MESSAGE_BODY.to_owned())
    );

    sqlx::query("UPDATE messages SET body = NULL WHERE id = $1")
        .bind(topology.message_id)
        .execute(&pool)
        .await?;
    assert_eq!(
        repository.load_message_body(topology.message_id).await?,
        None
    );
    assert_eq!(repository.load_message_body(Uuid::new_v4()).await?, None);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn device_not_registered_disables_only_the_exact_installation_and_terminalizes_the_claim()
-> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let topology = SendTopology::new(&pool).await?;
    let sibling_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO push_installations \
             (id, user_id, installation_id, platform, provider, token, environment, \
              message_preview_enabled) \
         VALUES ($1, $2, $3, 'ios', 'expo', $4, 'development', true)",
    )
    .bind(sibling_id)
    .bind(topology.recipient_id)
    .bind(format!("task-9-feedback-sibling-{sibling_id}"))
    .bind(format!(
        "ExponentPushToken[task-9-feedback-sibling-{sibling_id}]"
    ))
    .execute(&pool)
    .await?;

    let repository = PostgresPushRepository::new(pool.clone());
    let transactions = SqlxTransactionManager::new(pool.clone());
    let destination =
        ExpoPushDestination::new(PushEnvironment::Development, topology.expo_token.clone());
    assert!(committed_disable(&repository, &transactions, &topology.claim, &destination,).await?);

    let installations = sqlx::query_as::<_, (Uuid, bool)>(
        "SELECT id, disabled_at IS NOT NULL \
         FROM push_installations WHERE id = ANY($1::UUID[]) ORDER BY id",
    )
    .bind(vec![topology.installation_id, sibling_id])
    .fetch_all(&pool)
    .await?;
    assert_eq!(installations.len(), 2);
    assert!(installations.contains(&(topology.installation_id, true)));
    assert!(installations.contains(&(sibling_id, false)));

    let occurrence = sqlx::query_as::<_, (String, i32, Option<String>, bool, bool, bool)>(
        "SELECT status, attempt_count, last_error_code, failed_at IS NOT NULL, \
                claim_owner IS NULL, lease_expires_at IS NULL \
         FROM push_delivery_intents WHERE id = $1",
    )
    .bind(topology.occurrence_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        occurrence,
        (
            "failed".to_owned(),
            1,
            Some("device_not_registered".to_owned()),
            true,
            true,
            true,
        )
    );
    let notification_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM notifications WHERE id = $1")
            .bind(topology.notification_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(notification_count, 1);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn rebound_or_generation_stale_feedback_cannot_disable_the_current_destination() -> TestResult
{
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let repository = PostgresPushRepository::new(pool.clone());
    let transactions = SqlxTransactionManager::new(pool.clone());

    let rebound = SendTopology::new(&pool).await?;
    let old_destination =
        ExpoPushDestination::new(PushEnvironment::Development, rebound.expo_token.clone());
    let rotated_token = "ExponentPushToken[rotated-after-provider]".to_owned();
    let rotated = committed_update(
        &repository,
        &transactions,
        UpdatePushInstallationCommand {
            user_id: rebound.recipient_id,
            installation_id: rebound.public_installation_id.clone(),
            token: rotated_token.clone(),
            message_preview_enabled: None,
        },
    )
    .await?;
    assert_eq!(rotated.owner_epoch, 2);
    assert!(
        !committed_disable(&repository, &transactions, &rebound.claim, &old_destination,).await?
    );
    let rebound_state = sqlx::query_as::<_, (i64, String, bool)>(
        "SELECT owner_epoch, token, disabled_at IS NULL \
         FROM push_installations WHERE id = $1",
    )
    .bind(rebound.installation_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(rebound_state, (2, rotated_token, true));

    let newer_generation = SendTopology::new(&pool).await?;
    sqlx::query("UPDATE push_delivery_intents SET claim_generation = 2 WHERE id = $1")
        .bind(newer_generation.occurrence_id)
        .execute(&pool)
        .await?;
    let current_destination = ExpoPushDestination::new(
        PushEnvironment::Development,
        newer_generation.expo_token.clone(),
    );
    assert!(
        !committed_disable(
            &repository,
            &transactions,
            &newer_generation.claim,
            &current_destination,
        )
        .await?
    );
    let generation_state = sqlx::query_as::<_, (bool, String, i64)>(
        "SELECT installation.disabled_at IS NULL, intent.status, intent.claim_generation \
         FROM push_delivery_intents intent \
         JOIN push_installations installation ON installation.id = intent.push_installation_id \
         WHERE intent.id = $1",
    )
    .bind(newer_generation.occurrence_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(generation_state, (true, "claimed".to_owned(), 2));

    pool.close().await;
    database.dispose().await
}

async fn committed_disable(
    repository: &PostgresPushRepository,
    transactions: &SqlxTransactionManager,
    claim: &PushDeliveryClaim,
    destination: &ExpoPushDestination,
) -> TestResult<bool> {
    let mut transaction = transactions.begin().await?;
    let result = repository
        .disable_invalid_destination(transaction.as_mut(), claim, destination)
        .await;
    match result {
        Ok(true) => {
            transactions.commit(transaction).await?;
            Ok(true)
        }
        Ok(false) => {
            transactions.rollback(transaction).await?;
            Ok(false)
        }
        Err(error) => {
            transactions.rollback(transaction).await?;
            Err(error.into())
        }
    }
}
