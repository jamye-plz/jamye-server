//! `accounts delete` (SRV20C-AC7): the operator starts the same 30-day grace deletion the in-app
//! path uses, after verifying a web deletion request.

use jamye_server::application::account_deletion::AccountDeletionError;
use jamye_server::ports::account_deletion::AccountDeletionCommand;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    TestResult,
    harness::{AdminHarness, assert_failure, assert_usage, insert_session},
    support::{insert_group, insert_installation, insert_user},
};

/// A member of a group owned by someone else, with a session and a push installation.
async fn member_account(harness: &AdminHarness, nickname: &str) -> TestResult<(Uuid, Uuid)> {
    let owner = insert_user(&harness.pool, &format!("{nickname}-owner")).await?;
    let user = insert_user(&harness.pool, nickname).await?;
    let group = insert_group(&harness.pool, &[owner, user]).await?;
    insert_session(&harness.pool, user).await?;
    insert_installation(&harness.pool, user).await?;
    Ok((user, group.group_id))
}

/// The grace-deletion state of an account, comparable across the operator and in-app paths.
#[derive(Debug, Eq, PartialEq)]
struct GraceState {
    account_marked_deleted: bool,
    live_memberships: i64,
    live_sessions: i64,
    live_installations: i64,
    live_identities: i64,
    purged: bool,
}

async fn grace_state(pool: &PgPool, user: Uuid) -> TestResult<GraceState> {
    let account_marked_deleted =
        sqlx::query_scalar::<_, bool>("SELECT deleted_at IS NOT NULL FROM users WHERE id = $1")
            .bind(user)
            .fetch_one(pool)
            .await?;
    let live_memberships = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM memberships WHERE user_id = $1 AND deleted_at IS NULL",
    )
    .bind(user)
    .fetch_one(pool)
    .await?;
    let live_sessions = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM refresh_sessions \
         WHERE user_id = $1 AND deleted_at IS NULL AND revoked_at IS NULL",
    )
    .bind(user)
    .fetch_one(pool)
    .await?;
    let live_installations = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM push_installations WHERE user_id = $1 AND deleted_at IS NULL",
    )
    .bind(user)
    .fetch_one(pool)
    .await?;
    let live_identities = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM auth_identities WHERE user_id = $1 AND deleted_at IS NULL",
    )
    .bind(user)
    .fetch_one(pool)
    .await?;
    let purged = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM anonymous_author_tombstones WHERE user_id = $1",
    )
    .bind(user)
    .fetch_one(pool)
    .await?
        > 0;
    Ok(GraceState {
        account_marked_deleted,
        live_memberships,
        live_sessions,
        live_installations,
        live_identities,
        purged,
    })
}

async fn deleted_at(pool: &PgPool, user: Uuid) -> TestResult<Option<OffsetDateTime>> {
    Ok(sqlx::query_scalar::<_, Option<OffsetDateTime>>(
        "SELECT deleted_at FROM users WHERE id = $1",
    )
    .bind(user)
    .fetch_one(pool)
    .await?)
}

#[tokio::test]
async fn delete_starts_the_same_grace_deletion_as_the_in_app_path() -> TestResult {
    let harness = AdminHarness::new().await?;
    let result: TestResult = async {
        let (by_operator, _) = member_account(&harness, "operator-deleted").await?;
        let (in_app, _) = member_account(&harness, "self-deleted").await?;
        let before = grace_state(&harness.pool, by_operator).await?;
        assert!(!before.account_marked_deleted && before.live_memberships == 1);

        let output = harness
            .run(&["accounts", "delete", &by_operator.to_string()])
            .await?;
        assert!(output.contains("deletion started"), "{output}");
        assert!(output.contains("1 membership(s) removed"), "{output}");
        assert!(
            !output.contains("warning"),
            "a non-Apple account has no Apple limitation: {output}"
        );

        // The reference: the in-app service call (what DELETE /api/v1/me runs).
        harness
            .services
            .accounts
            .delete_account(AccountDeletionCommand {
                user_id: in_app,
                apple_proof: None,
            })
            .await?;

        let operator_state = grace_state(&harness.pool, by_operator).await?;
        let in_app_state = grace_state(&harness.pool, in_app).await?;
        assert_eq!(
            operator_state,
            GraceState {
                account_marked_deleted: true,
                live_memberships: 0,
                live_sessions: 0,
                live_installations: 0,
                live_identities: 0,
                purged: false,
            }
        );
        assert_eq!(operator_state, in_app_state);

        // Grace, not purge: the account row stays until the worker purges it after the grace
        // period, and the row carries the grace start time the worker reads.
        assert!(deleted_at(&harness.pool, by_operator).await?.is_some());

        // A second run reports the account as already handled and changes nothing.
        let marked = deleted_at(&harness.pool, by_operator).await?;
        assert_failure(
            harness
                .run(&["accounts", "delete", &by_operator.to_string()])
                .await,
            "not found, or its deletion already started",
        )?;
        assert_eq!(deleted_at(&harness.pool, by_operator).await?, marked);
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn an_apple_account_is_deleted_without_apple_revocation_and_the_operator_is_told()
-> TestResult {
    let harness = AdminHarness::new().await?;
    let result: TestResult = async {
        let (user, _) = member_account(&harness, "apple-user").await?;
        sqlx::query(
            "UPDATE auth_identities SET provider = 'apple', provider_id = $2 WHERE user_id = $1",
        )
        .bind(user)
        .bind(format!("apple-subject-{user}"))
        .execute(&harness.pool)
        .await?;

        // The in-app path refuses without the user's Sign in with Apple re-authentication.
        let refused = harness
            .services
            .accounts
            .delete_account(AccountDeletionCommand {
                user_id: user,
                apple_proof: None,
            })
            .await;
        assert_eq!(
            refused.err(),
            Some(AccountDeletionError::AppleReauthenticationRequired)
        );
        assert!(deleted_at(&harness.pool, user).await?.is_none());

        // The operator path cannot present that proof: it starts the deletion and says that the
        // Apple authorization stays.
        let output = harness
            .run(&["accounts", "delete", &user.to_string()])
            .await?;
        assert!(output.contains("deletion started"), "{output}");
        assert!(output.contains("warning"), "{output}");
        assert!(output.contains("NOT revoked"), "{output}");
        assert!(deleted_at(&harness.pool, user).await?.is_some());
        assert_eq!(grace_state(&harness.pool, user).await?.live_identities, 0);
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn an_owner_of_an_active_group_and_unknown_accounts_are_refused_without_changes() -> TestResult
{
    let harness = AdminHarness::new().await?;
    let result: TestResult = async {
        let owner = insert_user(&harness.pool, "owner").await?;
        let member = insert_user(&harness.pool, "member").await?;
        insert_group(&harness.pool, &[owner, member]).await?;

        assert_failure(
            harness
                .run(&["accounts", "delete", &owner.to_string()])
                .await,
            "owns an active group",
        )?;
        assert!(deleted_at(&harness.pool, owner).await?.is_none());

        assert_failure(
            harness
                .run(&["accounts", "delete", &Uuid::new_v4().to_string()])
                .await,
            "not found",
        )?;
        assert_usage(harness.run(&["accounts", "delete", "nope"]).await)?;
        assert_usage(harness.run(&["accounts", "delete"]).await)?;
        Ok(())
    }
    .await;
    harness.finish(result).await
}
