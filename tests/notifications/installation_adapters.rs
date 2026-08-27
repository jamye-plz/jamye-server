use jamye_server::{
    adapters::postgres::{push::PostgresPushRepository, transactions::SqlxTransactionManager},
    ports::{
        push::{
            DeletePushInstallationCommand, PushEnvironment, PushPlatform, PushProviderName,
            PushRepository, PushRepositoryError, UpdatePushInstallationCommand,
            UpsertPushInstallationCommand, UpsertPushInstallationOutcome,
        },
        transactions::TransactionManager,
    },
};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

const INSTALLATION_A: &str = "task-9-installation-a";
const INSTALLATION_B: &str = "task-9-installation-b";
const TOKEN_A: &str = "ExponentPushToken[task-9-a]";
const TOKEN_B: &str = "ExponentPushToken[task-9-b]";
const TOKEN_SHARED: &str = "ExponentPushToken[task-9-shared]";

#[tokio::test]
async fn postgres_p2_reuses_one_row_and_increments_epoch_without_inheriting_preview() -> TestResult
{
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let first_owner = insert_user(&pool, "첫 설치 소유자").await?;
    let next_owner = insert_user(&pool, "다음 설치 소유자").await?;
    let repository = PostgresPushRepository::new(pool.clone());
    let transactions = SqlxTransactionManager::new(pool.clone());

    let first = committed_upsert(
        &repository,
        &transactions,
        upsert_command(
            first_owner,
            INSTALLATION_A,
            TOKEN_A,
            PushEnvironment::Development,
            true,
        ),
    )
    .await?;
    assert!(first.created);
    assert_eq!(first.installation.owner_epoch, 1);
    assert!(first.installation.message_preview_enabled);

    let same_binding = committed_upsert(
        &repository,
        &transactions,
        upsert_command(
            first_owner,
            INSTALLATION_A,
            TOKEN_A,
            PushEnvironment::Development,
            true,
        ),
    )
    .await?;
    assert!(!same_binding.created);
    assert_eq!(same_binding.installation.id, first.installation.id);
    assert_eq!(same_binding.installation.owner_epoch, 1);

    let rebound = committed_upsert(
        &repository,
        &transactions,
        upsert_command(
            next_owner,
            INSTALLATION_A,
            TOKEN_B,
            PushEnvironment::Development,
            false,
        ),
    )
    .await?;
    assert!(!rebound.created);
    assert_eq!(rebound.installation.id, first.installation.id);
    assert_eq!(rebound.installation.user_id, next_owner);
    assert_eq!(rebound.installation.owner_epoch, 2);
    assert!(!rebound.installation.message_preview_enabled);
    assert_eq!(installation_count(&pool).await?, 1);

    let mut stale_update_transaction = transactions.begin().await?;
    let stale_update = repository
        .update_installation(
            stale_update_transaction.as_mut(),
            &UpdatePushInstallationCommand {
                user_id: first_owner,
                installation_id: INSTALLATION_A.to_owned(),
                token: TOKEN_A.to_owned(),
                message_preview_enabled: Some(true),
            },
        )
        .await;
    transactions.rollback(stale_update_transaction).await?;
    assert_eq!(stale_update, Err(PushRepositoryError::InstallationNotFound));

    let mut stale_delete_transaction = transactions.begin().await?;
    let stale_delete = repository
        .delete_installation(
            stale_delete_transaction.as_mut(),
            &DeletePushInstallationCommand {
                user_id: first_owner,
                installation_id: INSTALLATION_A.to_owned(),
            },
        )
        .await;
    transactions.rollback(stale_delete_transaction).await?;
    assert_eq!(stale_delete, Err(PushRepositoryError::InstallationNotFound));

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn postgres_p3_preserves_omitted_preview_and_p4_is_current_owner_scoped() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let owner_id = insert_user(&pool, "설치 관리자").await?;
    let outsider_id = insert_user(&pool, "오래된 소유자").await?;
    let repository = PostgresPushRepository::new(pool.clone());
    let transactions = SqlxTransactionManager::new(pool.clone());

    let created = committed_upsert(
        &repository,
        &transactions,
        upsert_command(
            owner_id,
            INSTALLATION_A,
            TOKEN_A,
            PushEnvironment::Development,
            true,
        ),
    )
    .await?;
    assert_eq!(created.installation.owner_epoch, 1);

    let preserved = committed_update(
        &repository,
        &transactions,
        UpdatePushInstallationCommand {
            user_id: owner_id,
            installation_id: INSTALLATION_A.to_owned(),
            token: TOKEN_A.to_owned(),
            message_preview_enabled: None,
        },
    )
    .await?;
    assert!(preserved.message_preview_enabled);
    assert_eq!(preserved.owner_epoch, 1);

    let rotated = committed_update(
        &repository,
        &transactions,
        UpdatePushInstallationCommand {
            user_id: owner_id,
            installation_id: INSTALLATION_A.to_owned(),
            token: TOKEN_B.to_owned(),
            message_preview_enabled: None,
        },
    )
    .await?;
    assert!(rotated.message_preview_enabled);
    assert_eq!(rotated.owner_epoch, 2);

    let disabled_preview = committed_update(
        &repository,
        &transactions,
        UpdatePushInstallationCommand {
            user_id: owner_id,
            installation_id: INSTALLATION_A.to_owned(),
            token: TOKEN_B.to_owned(),
            message_preview_enabled: Some(false),
        },
    )
    .await?;
    assert!(!disabled_preview.message_preview_enabled);
    assert_eq!(disabled_preview.owner_epoch, 2);

    let moved = committed_upsert(
        &repository,
        &transactions,
        upsert_command(
            owner_id,
            INSTALLATION_A,
            TOKEN_A,
            PushEnvironment::Production,
            false,
        ),
    )
    .await?;
    assert!(!moved.created);
    assert_eq!(moved.installation.id, created.installation.id);
    assert_eq!(moved.installation.owner_epoch, 3);
    assert_eq!(moved.installation.environment, PushEnvironment::Production);

    let mut outsider_transaction = transactions.begin().await?;
    let outsider_delete = repository
        .delete_installation(
            outsider_transaction.as_mut(),
            &DeletePushInstallationCommand {
                user_id: outsider_id,
                installation_id: INSTALLATION_A.to_owned(),
            },
        )
        .await;
    transactions.rollback(outsider_transaction).await?;
    assert_eq!(
        outsider_delete,
        Err(PushRepositoryError::InstallationNotFound)
    );

    let mut owner_transaction = transactions.begin().await?;
    repository
        .delete_installation(
            owner_transaction.as_mut(),
            &DeletePushInstallationCommand {
                user_id: owner_id,
                installation_id: INSTALLATION_A.to_owned(),
            },
        )
        .await?;
    transactions.commit(owner_transaction).await?;
    assert_eq!(installation_count(&pool).await?, 0);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn postgres_p2_destination_collision_converges_to_one_new_identity_and_owner() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let first_owner = insert_user(&pool, "기존 destination 소유자").await?;
    let next_owner = insert_user(&pool, "새 destination 소유자").await?;
    let repository = PostgresPushRepository::new(pool.clone());
    let transactions = SqlxTransactionManager::new(pool.clone());

    let first = committed_upsert(
        &repository,
        &transactions,
        upsert_command(
            first_owner,
            INSTALLATION_A,
            TOKEN_SHARED,
            PushEnvironment::Development,
            true,
        ),
    )
    .await?;
    let converged = committed_upsert(
        &repository,
        &transactions,
        upsert_command(
            next_owner,
            INSTALLATION_B,
            TOKEN_SHARED,
            PushEnvironment::Development,
            false,
        ),
    )
    .await?;

    assert!(!converged.created);
    assert_eq!(converged.installation.id, first.installation.id);
    assert_eq!(converged.installation.installation_id, INSTALLATION_B);
    assert_eq!(converged.installation.user_id, next_owner);
    assert_eq!(converged.installation.owner_epoch, 2);
    assert!(!converged.installation.message_preview_enabled);
    assert_eq!(installation_count(&pool).await?, 1);
    let stored: (String, Uuid, i64, bool) = sqlx::query_as(
        "SELECT installation_id, user_id, owner_epoch, message_preview_enabled \
         FROM push_installations",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(stored, (INSTALLATION_B.to_owned(), next_owner, 2, false));

    let mut old_identity_transaction = transactions.begin().await?;
    let old_identity = repository
        .update_installation(
            old_identity_transaction.as_mut(),
            &UpdatePushInstallationCommand {
                user_id: first_owner,
                installation_id: INSTALLATION_A.to_owned(),
                token: TOKEN_A.to_owned(),
                message_preview_enabled: None,
            },
        )
        .await;
    transactions.rollback(old_identity_transaction).await?;
    assert_eq!(old_identity, Err(PushRepositoryError::InstallationNotFound));

    pool.close().await;
    database.dispose().await
}

async fn committed_upsert(
    repository: &PostgresPushRepository,
    transactions: &SqlxTransactionManager,
    command: UpsertPushInstallationCommand,
) -> TestResult<UpsertPushInstallationOutcome> {
    let mut transaction = transactions.begin().await?;
    let outcome = repository
        .upsert_installation(transaction.as_mut(), &command)
        .await?;
    transactions.commit(transaction).await?;
    Ok(outcome)
}

async fn committed_update(
    repository: &PostgresPushRepository,
    transactions: &SqlxTransactionManager,
    command: UpdatePushInstallationCommand,
) -> TestResult<jamye_server::ports::push::PushInstallationRecord> {
    let mut transaction = transactions.begin().await?;
    let installation = repository
        .update_installation(transaction.as_mut(), &command)
        .await?;
    transactions.commit(transaction).await?;
    Ok(installation)
}

fn upsert_command(
    user_id: Uuid,
    installation_id: &str,
    token: &str,
    environment: PushEnvironment,
    message_preview_enabled: bool,
) -> UpsertPushInstallationCommand {
    UpsertPushInstallationCommand {
        id: Uuid::new_v4(),
        user_id,
        installation_id: installation_id.to_owned(),
        platform: PushPlatform::Ios,
        provider: PushProviderName::Expo,
        token: token.to_owned(),
        environment,
        message_preview_enabled,
    }
}

async fn insert_user(pool: &PgPool, nickname: &str) -> TestResult<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, $2)")
        .bind(id)
        .bind(nickname)
        .execute(pool)
        .await?;
    Ok(id)
}

async fn installation_count(pool: &PgPool) -> TestResult<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM push_installations")
            .fetch_one(pool)
            .await?,
    )
}
