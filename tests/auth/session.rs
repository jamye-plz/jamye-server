use std::sync::Arc;

use jamye_server::application::auth::{AccessTokenVerifier, AuthError};
use tokio::sync::Barrier;

use crate::{
    TestResult,
    auth_helpers::{authorize, exchange, harness},
    postgres_support::TestDatabase,
};

#[tokio::test]
async fn concurrent_callbacks_converge_on_one_identity_and_user() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone(), Some(Arc::new(Barrier::new(2))))?;
    let first_state = authorize(&fixture.service).await?;
    let second_state = authorize(&fixture.service).await?;

    let (first, second) = tokio::join!(
        exchange(&fixture.service, first_state),
        exchange(&fixture.service, second_state)
    );
    assert!(first.is_ok());
    assert!(second.is_ok());
    assert_eq!(fixture.attempts.len()?, 0);

    let user_count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&pool)
        .await?;
    let identity_count: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_identities")
        .fetch_one(&pool)
        .await?;
    let session_count: i64 = sqlx::query_scalar("SELECT count(*) FROM refresh_sessions")
        .fetch_one(&pool)
        .await?;
    assert_eq!(user_count, 1);
    assert_eq!(identity_count, 1);
    assert_eq!(session_count, 2);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn concurrent_refresh_allows_one_child_then_reuse_revokes_the_family() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone(), None)?;
    let state = authorize(&fixture.service).await?;
    let initial = exchange(&fixture.service, state).await?;

    let (first, second) = tokio::join!(
        fixture.service.refresh(&initial.refresh_token),
        fixture.service.refresh(&initial.refresh_token)
    );
    let child = match (first, second) {
        (Ok(child), Err(AuthError::RefreshTokenReused))
        | (Err(AuthError::RefreshTokenReused), Ok(child)) => child,
        outcomes => return Err(format!("unexpected refresh race outcomes: {outcomes:?}").into()),
    };
    assert_eq!(
        fixture.service.refresh(&child.refresh_token).await,
        Err(AuthError::RefreshTokenInvalid)
    );

    let row = sqlx::query_as::<_, (i64, i64, i64)>(
        "SELECT count(*), \
                count(*) FILTER (WHERE parent_session_id IS NOT NULL), \
                count(*) FILTER (WHERE revoked_at IS NOT NULL) \
         FROM refresh_sessions",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(row, (2, 1, 2));

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn failed_child_insert_rolls_back_parent_consumption_and_allows_retry() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone(), None)?;
    let initial = exchange(&fixture.service, authorize(&fixture.service).await?).await?;

    sqlx::query(
        "ALTER TABLE refresh_sessions \
         ADD CONSTRAINT task_5_force_child_failure CHECK (parent_session_id IS NULL)",
    )
    .execute(&pool)
    .await?;
    assert_eq!(
        fixture.service.refresh(&initial.refresh_token).await,
        Err(AuthError::DatabaseUnavailable)
    );
    let parent = sqlx::query_as::<_, (bool, bool)>(
        "SELECT consumed_at IS NOT NULL, revoked_at IS NOT NULL \
         FROM refresh_sessions WHERE parent_session_id IS NULL",
    )
    .fetch_one(&pool)
    .await?;
    let session_count: i64 = sqlx::query_scalar("SELECT count(*) FROM refresh_sessions")
        .fetch_one(&pool)
        .await?;
    assert_eq!(parent, (false, false));
    assert_eq!(session_count, 1);

    sqlx::query("ALTER TABLE refresh_sessions DROP CONSTRAINT task_5_force_child_failure")
        .execute(&pool)
        .await?;
    assert!(
        fixture
            .service
            .refresh(&initial.refresh_token)
            .await
            .is_ok()
    );

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn logout_revokes_only_refresh_authority_and_access_remains_valid_to_expiry() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone(), None)?;
    let first = exchange(&fixture.service, authorize(&fixture.service).await?).await?;
    let second = exchange(&fixture.service, authorize(&fixture.service).await?).await?;
    let first_identity = fixture.codec.verify(&first.access_token)?;
    let second_identity = fixture.codec.verify(&second.access_token)?;
    assert_eq!(first_identity.user_id, second_identity.user_id);
    assert_ne!(first_identity.session_id, second_identity.session_id);

    fixture
        .service
        .logout(first_identity.session_id, "user:fixture")
        .await?;
    fixture
        .service
        .logout(first_identity.session_id, "user:fixture")
        .await?;
    assert_eq!(
        fixture.service.refresh(&first.refresh_token).await,
        Err(AuthError::RefreshTokenInvalid)
    );
    assert!(fixture.codec.verify(&first.access_token).is_ok());
    assert!(fixture.service.refresh(&second.refresh_token).await.is_ok());

    let revoked: Vec<(uuid::Uuid, bool)> = sqlx::query_as(
        "SELECT id, revoked_at IS NOT NULL FROM refresh_sessions \
         WHERE id IN ($1, $2) ORDER BY id",
    )
    .bind(first_identity.session_id)
    .bind(second_identity.session_id)
    .fetch_all(&pool)
    .await?;
    assert_eq!(revoked.len(), 2);
    assert!(
        revoked
            .iter()
            .any(|(id, is_revoked)| *id == first_identity.session_id && *is_revoked)
    );
    assert!(
        revoked
            .iter()
            .any(|(id, is_revoked)| *id == second_identity.session_id && !*is_revoked)
    );

    pool.close().await;
    database.dispose().await
}
