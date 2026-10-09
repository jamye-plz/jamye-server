//! Suspension at OAuth exchange (Kakao and Apple) and at refresh, against the real PostgreSQL
//! repository: 403 `account_suspended`, suspend revokes sessions, unsuspend then sign-in works.

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use jamye_server::{
    adapters::postgres::{
        moderation::PostgresModerationRepository, transactions::SqlxTransactionManager,
    },
    application::auth::{AppleExchangeInput, AuthError},
    ports::{
        apple_identity_provider::AppleIdentity,
        moderation::{ModerationRepository, SuspendUserCommand},
        transactions::TransactionManager,
    },
    transport::http::auth::{AuthHttpState, router as auth_router},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    TestResult,
    auth_helpers::{
        KAKAO_REDIRECT, TEST_VERIFIER, authorize, exchange, harness, harness_with_apple_identity,
    },
    postgres_support::TestDatabase,
};

async fn suspend(pool: &PgPool, user_id: Uuid) -> TestResult {
    let repository = PostgresModerationRepository::new(pool.clone());
    let transactions = SqlxTransactionManager::new(pool.clone());
    let mut transaction = transactions.begin().await?;
    repository
        .suspend_user(
            transaction.as_mut(),
            &SuspendUserCommand {
                user_id,
                reason: None,
            },
        )
        .await?;
    transactions.commit(transaction).await?;
    Ok(())
}

async fn unsuspend(pool: &PgPool, user_id: Uuid) -> TestResult {
    let repository = PostgresModerationRepository::new(pool.clone());
    let transactions = SqlxTransactionManager::new(pool.clone());
    let mut transaction = transactions.begin().await?;
    repository
        .unsuspend_user(transaction.as_mut(), user_id)
        .await?;
    transactions.commit(transaction).await?;
    Ok(())
}

async fn identity_user(pool: &PgPool, provider: &str, provider_id: &str) -> TestResult<Uuid> {
    Ok(sqlx::query_scalar::<_, Uuid>(
        "SELECT user_id FROM auth_identities WHERE provider = $1 AND provider_id = $2",
    )
    .bind(provider)
    .bind(provider_id)
    .fetch_one(pool)
    .await?)
}

async fn session_counts(pool: &PgPool, user_id: Uuid) -> TestResult<(i64, i64)> {
    Ok(sqlx::query_as::<_, (i64, i64)>(
        "SELECT count(*), count(*) FILTER (WHERE revoked_at IS NULL) \
         FROM refresh_sessions WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?)
}

fn json_post(uri: &str, body: &Value) -> TestResult<Request<Body>> {
    Ok(Request::post(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(body)?))?)
}

async fn assert_suspended(response: axum::response::Response) -> TestResult {
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 4096).await?)?;
    assert_eq!(body["error"]["code"], "account_suspended");
    assert!(body["error"]["details"].is_null());
    Ok(())
}

#[tokio::test]
async fn kakao_exchange_is_refused_for_a_suspended_account_and_works_again_after_unsuspend()
-> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone(), None)?;
    let router = auth_router(AuthHttpState::new(
        fixture.service.clone(),
        fixture.codec.clone(),
    ));

    let state = authorize(&fixture.service).await?;
    exchange(&fixture.service, state).await?;
    let user_id = identity_user(&pool, "kakao", "kakao-principal-42").await?;
    assert_eq!(session_counts(&pool, user_id).await?, (1, 1));

    suspend(&pool, user_id).await?;
    assert_eq!(
        session_counts(&pool, user_id).await?,
        (1, 0),
        "suspend must revoke the account's sessions"
    );

    let state = authorize(&fixture.service).await?;
    let response = router
        .clone()
        .oneshot(json_post(
            "/api/v1/auth/oauth/kakao/exchange",
            &json!({
                "authorization_code": "provider-code",
                "state": state,
                "code_verifier": TEST_VERIFIER,
                "redirect_uri": KAKAO_REDIRECT,
            }),
        )?)
        .await?;
    assert_suspended(response).await?;
    assert_eq!(
        session_counts(&pool, user_id).await?,
        (1, 0),
        "a refused exchange must not issue a session"
    );

    unsuspend(&pool, user_id).await?;
    let state = authorize(&fixture.service).await?;
    exchange(&fixture.service, state).await?;
    assert_eq!(
        identity_user(&pool, "kakao", "kakao-principal-42").await?,
        user_id
    );
    assert_eq!(session_counts(&pool, user_id).await?, (2, 1));

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn apple_exchange_shares_the_refusal_and_the_restore() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness_with_apple_identity(
        pool.clone(),
        AppleIdentity {
            provider_id: "apple-subject-42".to_owned(),
            client_id: "dev.local.jamyeapp".to_owned(),
        },
    )?;
    let router = auth_router(AuthHttpState::new(
        fixture.service.clone(),
        fixture.codec.clone(),
    ));
    let input = |token: &str, nonce: &str| AppleExchangeInput {
        identity_token: token.to_owned(),
        raw_nonce: nonce.to_owned(),
        full_name: None,
    };

    fixture
        .service
        .exchange_apple_identity(
            input("APPLE_FIRST_TOKEN", "raw-nonce-111111"),
            "ip:apple-suspension",
        )
        .await?;
    let user_id = identity_user(&pool, "apple", "apple-subject-42").await?;

    suspend(&pool, user_id).await?;
    assert_eq!(
        fixture
            .service
            .exchange_apple_identity(
                input("APPLE_SECOND_TOKEN", "raw-nonce-222222"),
                "ip:apple-suspension"
            )
            .await,
        Err(AuthError::AccountSuspended)
    );
    let response = router
        .oneshot(json_post(
            "/api/v1/auth/apple/exchange",
            &json!({"identity_token": "APPLE_THIRD_TOKEN", "raw_nonce": "raw-nonce-333333"}),
        )?)
        .await?;
    assert_suspended(response).await?;
    assert_eq!(session_counts(&pool, user_id).await?, (1, 0));

    unsuspend(&pool, user_id).await?;
    fixture
        .service
        .exchange_apple_identity(
            input("APPLE_FOURTH_TOKEN", "raw-nonce-444444"),
            "ip:apple-suspension",
        )
        .await?;
    assert_eq!(
        identity_user(&pool, "apple", "apple-subject-42").await?,
        user_id
    );
    assert_eq!(session_counts(&pool, user_id).await?, (2, 1));

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn refresh_answers_403_while_suspended_and_unsuspend_needs_a_new_sign_in() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone(), None)?;
    let router = auth_router(AuthHttpState::new(
        fixture.service.clone(),
        fixture.codec.clone(),
    ));

    let state = authorize(&fixture.service).await?;
    let pair = exchange(&fixture.service, state).await?;
    let user_id = identity_user(&pool, "kakao", "kakao-principal-42").await?;

    suspend(&pool, user_id).await?;
    assert_eq!(session_counts(&pool, user_id).await?, (1, 0));

    // The refusal does not consume the token, so it stays 403 for as long as the account is
    // suspended, on the service and on the HTTP route.
    for _ in 0..2 {
        assert_eq!(
            fixture.service.refresh(&pair.refresh_token).await,
            Err(AuthError::AccountSuspended)
        );
    }
    let response = router
        .oneshot(json_post(
            "/api/v1/auth/refresh",
            &json!({"refresh_token": pair.refresh_token}),
        )?)
        .await?;
    assert_suspended(response).await?;

    // Unsuspend restores sign-in, not the revoked session.
    unsuspend(&pool, user_id).await?;
    assert_eq!(
        fixture.service.refresh(&pair.refresh_token).await,
        Err(AuthError::RefreshTokenInvalid)
    );
    let state = authorize(&fixture.service).await?;
    let again = exchange(&fixture.service, state).await?;
    assert!(fixture.service.refresh(&again.refresh_token).await.is_ok());

    pool.close().await;
    database.dispose().await
}
