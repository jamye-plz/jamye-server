//! `users suspend|unsuspend` call the 20b moderation service.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    TestResult,
    harness::{AdminHarness, assert_failure, assert_usage, insert_session},
    support::insert_user,
};

async fn suspension(
    harness: &AdminHarness,
    user: Uuid,
) -> TestResult<(Option<OffsetDateTime>, Option<String>)> {
    Ok(
        sqlx::query_as::<_, (Option<OffsetDateTime>, Option<String>)>(
            "SELECT suspended_at, suspension_reason FROM users WHERE id = $1",
        )
        .bind(user)
        .fetch_one(&harness.pool)
        .await?,
    )
}

#[tokio::test]
async fn suspend_sets_the_suspension_revokes_sessions_and_unsuspend_clears_it() -> TestResult {
    let harness = AdminHarness::new().await?;
    let result: TestResult = async {
        let user = insert_user(&harness.pool, "suspect").await?;
        let other = insert_user(&harness.pool, "bystander").await?;
        let session = insert_session(&harness.pool, user).await?;
        let other_session = insert_session(&harness.pool, other).await?;

        let output = harness
            .run(&[
                "users",
                "suspend",
                &user.to_string(),
                "--reason",
                "zero tolerance",
            ])
            .await?;
        assert!(
            output.contains(&format!("user {user} suspended")),
            "{output}"
        );
        let (suspended_at, reason) = suspension(&harness, user).await?;
        assert!(suspended_at.is_some());
        assert_eq!(reason.as_deref(), Some("zero tolerance"));
        let revoked = sqlx::query_scalar::<_, bool>(
            "SELECT revoked_at IS NOT NULL FROM refresh_sessions WHERE id = $1",
        )
        .bind(session)
        .fetch_one(&harness.pool)
        .await?;
        assert!(revoked, "suspend must revoke the refresh sessions");
        let untouched = sqlx::query_scalar::<_, bool>(
            "SELECT revoked_at IS NULL FROM refresh_sessions WHERE id = $1",
        )
        .bind(other_session)
        .fetch_one(&harness.pool)
        .await?;
        assert!(untouched, "another account's session must stay valid");
        assert!(suspension(&harness, other).await?.0.is_none());

        // Idempotent: the first suspension time stays.
        let first = suspended_at;
        harness
            .run(&["users", "suspend", &user.to_string()])
            .await?;
        assert_eq!(suspension(&harness, user).await?.0, first);

        let output = harness
            .run(&["users", "unsuspend", &user.to_string()])
            .await?;
        assert!(output.contains("unsuspended"), "{output}");
        assert_eq!(suspension(&harness, user).await?, (None, None));
        harness
            .run(&["users", "unsuspend", &user.to_string()])
            .await?;
        assert_eq!(suspension(&harness, user).await?, (None, None));
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn unknown_users_and_bad_reasons_are_refused() -> TestResult {
    let harness = AdminHarness::new().await?;
    let result: TestResult = async {
        let unknown = Uuid::new_v4().to_string();
        assert_failure(
            harness.run(&["users", "suspend", &unknown]).await,
            "not found",
        )?;
        assert_failure(
            harness.run(&["users", "unsuspend", &unknown]).await,
            "not found",
        )?;

        let user = insert_user(&harness.pool, "suspect").await?;
        let too_long = "x".repeat(501);
        assert_usage(
            harness
                .run(&["users", "suspend", &user.to_string(), "--reason", &too_long])
                .await,
        )?;
        assert_usage(
            harness
                .run(&[
                    "users",
                    "suspend",
                    &user.to_string(),
                    "--reason",
                    "bad\u{7}reason",
                ])
                .await,
        )?;
        assert_usage(
            harness
                .run(&["users", "suspend", &user.to_string(), "--reason"])
                .await,
        )?;
        assert_usage(harness.run(&["users", "suspend", "nope"]).await)?;
        assert_eq!(suspension(&harness, user).await?, (None, None));
        Ok(())
    }
    .await;
    harness.finish(result).await
}
