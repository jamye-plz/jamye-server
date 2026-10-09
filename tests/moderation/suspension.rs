//! Suspension: `403 account_suspended` at the bearer extractor, refresh, OAuth exchange and
//! realtime ticket/connection; suspend revokes sessions; unsuspend restores sign-in.

use axum::http::StatusCode;
use jamye_server::{
    adapters::postgres::{auth::PostgresAuthRepository, transactions::SqlxTransactionManager},
    ports::{
        auth::{
            AuthRepository, AuthRepositoryError, CredentialDigest, NewProviderIdentity,
            NewRefreshSession, NewRotatedSession, RotationOutcome,
        },
        transactions::TransactionManager,
    },
};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{
    TestResult,
    support::{Harness, assert_error, insert_user},
};

#[tokio::test]
async fn a_suspended_account_gets_403_account_suspended_at_the_bearer_extractor() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let user = insert_user(&harness.pool, "suspended").await?;
        let response = harness.list_blocks(user).await?;
        assert_eq!(response.status(), StatusCode::OK);

        harness
            .service
            .suspend_user(user, Some("zero tolerance".to_owned()))
            .await?;
        let (suspended, reason) = sqlx::query_as::<_, (bool, Option<String>)>(
            "SELECT suspended_at IS NOT NULL, suspension_reason FROM users WHERE id = $1",
        )
        .bind(user)
        .fetch_one(&harness.pool)
        .await?;
        assert!(suspended);
        assert_eq!(reason.as_deref(), Some("zero tolerance"));

        for response in [
            harness.list_blocks(user).await?,
            harness.put_block(user, &Uuid::new_v4().to_string()).await?,
            harness
                .report(
                    user,
                    &crate::support::report_user_body(Uuid::new_v4(), "spam"),
                )
                .await?,
        ] {
            assert_error(response, StatusCode::FORBIDDEN, "account_suspended").await?;
        }

        // Other accounts are unaffected, and unsuspend restores access.
        let other = insert_user(&harness.pool, "other").await?;
        assert_eq!(harness.list_blocks(other).await?.status(), StatusCode::OK);
        harness.service.unsuspend_user(user).await?;
        assert_eq!(harness.list_blocks(user).await?.status(), StatusCode::OK);
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn suspend_is_idempotent_and_unknown_accounts_are_not_found() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let user = insert_user(&harness.pool, "suspended").await?;
        harness.service.suspend_user(user, None).await?;
        let first = sqlx::query_scalar::<_, time::OffsetDateTime>(
            "SELECT suspended_at FROM users WHERE id = $1",
        )
        .bind(user)
        .fetch_one(&harness.pool)
        .await?;
        harness
            .service
            .suspend_user(user, Some("later reason".to_owned()))
            .await?;
        let second = sqlx::query_scalar::<_, time::OffsetDateTime>(
            "SELECT suspended_at FROM users WHERE id = $1",
        )
        .bind(user)
        .fetch_one(&harness.pool)
        .await?;
        assert_eq!(
            first, second,
            "a repeated suspend keeps the first timestamp"
        );
        harness.service.unsuspend_user(user).await?;
        harness.service.unsuspend_user(user).await?;
        let cleared = sqlx::query_as::<_, (Option<time::OffsetDateTime>, Option<String>)>(
            "SELECT suspended_at, suspension_reason FROM users WHERE id = $1",
        )
        .bind(user)
        .fetch_one(&harness.pool)
        .await?;
        assert_eq!(cleared, (None, None));
        assert!(
            harness
                .service
                .suspend_user(Uuid::new_v4(), None)
                .await
                .is_err()
        );
        Ok(())
    }
    .await;
    harness.finish(result).await
}

fn digest(seed: u8) -> CredentialDigest {
    CredentialDigest::new([seed; 32])
}

fn new_session(seed: u8) -> NewRefreshSession {
    NewRefreshSession {
        id: Uuid::new_v4(),
        family_id: Uuid::new_v4(),
        parent_session_id: None,
        token_hash: digest(seed),
        expires_at: OffsetDateTime::now_utc() + Duration::days(30),
    }
}

#[tokio::test]
async fn suspend_revokes_sessions_refresh_answers_suspended_and_unsuspend_requires_sign_in()
-> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let auth = PostgresAuthRepository::new(harness.pool.clone());
        let transactions = SqlxTransactionManager::new(harness.pool.clone());
        let user = insert_user(&harness.pool, "refresher").await?;
        let session = new_session(1);
        sqlx::query(
            "INSERT INTO refresh_sessions (id, user_id, family_id, token_hash, expires_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(session.id)
        .bind(user)
        .bind(session.family_id)
        .bind(session.token_hash.as_bytes().as_slice())
        .bind(session.expires_at)
        .execute(&harness.pool)
        .await?;

        harness.service.suspend_user(user, None).await?;
        let revoked = sqlx::query_scalar::<_, bool>(
            "SELECT revoked_at IS NOT NULL FROM refresh_sessions WHERE id = $1",
        )
        .bind(session.id)
        .fetch_one(&harness.pool)
        .await?;
        assert!(revoked, "suspend must revoke refresh sessions");

        let mut transaction = transactions.begin().await?;
        let outcome = auth
            .rotate_session(
                transaction.as_mut(),
                &digest(1),
                &NewRotatedSession {
                    id: Uuid::new_v4(),
                    token_hash: digest(2),
                    expires_at: OffsetDateTime::now_utc() + Duration::days(30),
                },
                OffsetDateTime::now_utc(),
            )
            .await?;
        transactions.rollback(transaction).await?;
        assert_eq!(outcome, RotationOutcome::Suspended);

        // After unsuspend the revoked token stays unusable: the user must sign in again.
        harness.service.unsuspend_user(user).await?;
        let mut transaction = transactions.begin().await?;
        let outcome = auth
            .rotate_session(
                transaction.as_mut(),
                &digest(1),
                &NewRotatedSession {
                    id: Uuid::new_v4(),
                    token_hash: digest(3),
                    expires_at: OffsetDateTime::now_utc() + Duration::days(30),
                },
                OffsetDateTime::now_utc(),
            )
            .await?;
        transactions.rollback(transaction).await?;
        assert_eq!(outcome, RotationOutcome::Invalid);
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn oauth_exchange_is_refused_for_a_suspended_account_and_works_after_unsuspend() -> TestResult
{
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let auth = PostgresAuthRepository::new(harness.pool.clone());
        let transactions = SqlxTransactionManager::new(harness.pool.clone());
        let identity = NewProviderIdentity {
            provider: "kakao".to_owned(),
            provider_id: format!("suspended-{}", Uuid::new_v4()),
            nickname: "returning".to_owned(),
            avatar_url: None,
        };

        let mut transaction = transactions.begin().await?;
        let issued = auth
            .create_session(transaction.as_mut(), &identity, &new_session(10))
            .await?;
        transactions.commit(transaction).await?;

        harness.service.suspend_user(issued.user_id, None).await?;
        let mut transaction = transactions.begin().await?;
        let refused = auth
            .create_session(transaction.as_mut(), &identity, &new_session(11))
            .await;
        transactions.rollback(transaction).await?;
        assert_eq!(refused.err(), Some(AuthRepositoryError::AccountSuspended));

        harness.service.unsuspend_user(issued.user_id).await?;
        let mut transaction = transactions.begin().await?;
        let again = auth
            .create_session(transaction.as_mut(), &identity, &new_session(12))
            .await?;
        transactions.commit(transaction).await?;
        assert_eq!(again.user_id, issued.user_id);
        Ok(())
    }
    .await;
    harness.finish(result).await
}
