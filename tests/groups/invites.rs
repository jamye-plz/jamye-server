use std::sync::Arc;

use jamye_server::{
    application::groups::{GroupsError, InviteCreateInput},
    ports::groups::GroupRole,
};

use crate::{
    TestResult,
    groups_helpers::{
        UnavailableRateLimiter, create_group, create_invite, harness, harness_with_limiter,
        insert_member, insert_user, test_now,
    },
    postgres_support::TestDatabase,
};

#[tokio::test]
async fn existing_member_precedes_expiry_and_exhaustion_without_consuming_a_use() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "기존 멤버").await?;
    let group = create_group(&fixture, owner_id).await?;
    let invite = create_invite(&fixture, owner_id, group.id, Some(1)).await?;
    sqlx::query("UPDATE invites SET expires_at = $2, used_count = 1 WHERE id = $1")
        .bind(invite.id)
        .bind(test_now() - time::Duration::hours(1))
        .execute(&pool)
        .await?;

    let result = fixture
        .service
        .redeem_invite(owner_id, invite.code, "existing-member")
        .await?;
    assert!(!result.joined);
    assert_eq!(result.membership_id, None);
    assert_eq!(invite_use_count(&pool, invite.id).await?, 1);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn concurrent_same_user_redeems_converge_on_one_membership_and_one_use() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "동시성 소유자").await?;
    let joiner_id = insert_user(&pool, "동일 가입자").await?;
    let group = create_group(&fixture, owner_id).await?;
    let invite = create_invite(&fixture, owner_id, group.id, Some(2)).await?;

    let first_service = fixture.service.clone();
    let second_service = fixture.service.clone();
    let first_code = invite.code.clone();
    let second_code = invite.code.clone();
    let (first, second) = tokio::join!(
        async move {
            first_service
                .redeem_invite(joiner_id, first_code, "same-user:first")
                .await
        },
        async move {
            second_service
                .redeem_invite(joiner_id, second_code, "same-user:second")
                .await
        }
    );
    let outcomes = [first?, second?];
    assert_eq!(outcomes.iter().filter(|outcome| outcome.joined).count(), 1);
    assert_eq!(outcomes.iter().filter(|outcome| !outcome.joined).count(), 1);
    assert_eq!(membership_count(&pool, group.id, joiner_id).await?, 1);
    assert_eq!(invite_use_count(&pool, invite.id).await?, 1);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn concurrent_distinct_users_never_exceed_global_max_uses() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "사용량 소유자").await?;
    let first_user = insert_user(&pool, "첫 가입자").await?;
    let second_user = insert_user(&pool, "둘째 가입자").await?;
    let group = create_group(&fixture, owner_id).await?;
    let invite = create_invite(&fixture, owner_id, group.id, Some(1)).await?;

    let first_service = fixture.service.clone();
    let second_service = fixture.service.clone();
    let first_code = invite.code.clone();
    let second_code = invite.code.clone();
    let (first, second) = tokio::join!(
        async move {
            first_service
                .redeem_invite(first_user, first_code, "max-use:first")
                .await
        },
        async move {
            second_service
                .redeem_invite(second_user, second_code, "max-use:second")
                .await
        }
    );
    let outcomes = [first, second];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Ok(joined) if joined.joined))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Err(GroupsError::InviteExhausted)))
            .count(),
        1
    );
    let joined_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM memberships WHERE group_id = $1 AND user_id IN ($2, $3)",
    )
    .bind(group.id)
    .bind(first_user)
    .bind(second_user)
    .fetch_one(&pool)
    .await?;
    assert_eq!(joined_count, 1);
    assert_eq!(invite_use_count(&pool, invite.id).await?, 1);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn concurrent_group_delete_or_redeem_has_no_partial_membership_or_use() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "삭제 경합 소유자").await?;
    let joiner_id = insert_user(&pool, "삭제 경합 가입자").await?;
    let group = create_group(&fixture, owner_id).await?;
    let invite = create_invite(&fixture, owner_id, group.id, Some(1)).await?;
    let group_id = group.id;

    let delete_service = fixture.service.clone();
    let redeem_service = fixture.service.clone();
    let code = invite.code.clone();
    let (deleted, redeemed) = tokio::join!(
        async move { delete_service.delete_group(owner_id, group_id).await },
        async move {
            redeem_service
                .redeem_invite(joiner_id, code, "delete-race")
                .await
        }
    );
    assert_eq!(deleted, Ok(()));
    let membership_count = membership_count(&pool, group_id, joiner_id).await?;
    let used_count = invite_use_count(&pool, invite.id).await?;
    match redeemed {
        Ok(result) => {
            assert!(result.joined);
            assert_eq!((membership_count, used_count), (1, 1));
        }
        Err(GroupsError::GroupNotFound) => {
            assert_eq!((membership_count, used_count), (0, 0));
        }
        other => return Err(format!("unexpected delete/redeem outcome: {other:?}").into()),
    }

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn default_member_cap_rejects_join_without_consuming_the_invite() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "정원 소유자").await?;
    let group = create_group(&fixture, owner_id).await?;
    for index in 0..11 {
        let user_id = insert_user(&pool, &format!("정원 멤버 {index}")).await?;
        insert_member(&pool, group.id, user_id, GroupRole::Member).await?;
    }
    let blocked_id = insert_user(&pool, "정원 밖 가입자").await?;
    let invite = create_invite(&fixture, owner_id, group.id, None).await?;

    assert_eq!(
        fixture
            .service
            .redeem_invite(blocked_id, invite.code, "group-full")
            .await,
        Err(GroupsError::GroupFull)
    );
    assert_eq!(membership_count(&pool, group.id, blocked_id).await?, 0);
    assert_eq!(invite_use_count(&pool, invite.id).await?, 0);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn unavailable_shared_limiter_fails_before_issue_or_redeem_mutation() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let allowed = harness(pool.clone())?;
    let unavailable = harness_with_limiter(pool.clone(), Arc::new(UnavailableRateLimiter))?;
    let owner_id = insert_user(&pool, "제한 소유자").await?;
    let joiner_id = insert_user(&pool, "제한 가입자").await?;
    let group = create_group(&allowed, owner_id).await?;
    let invite = create_invite(&allowed, owner_id, group.id, Some(1)).await?;
    let invite_count_before: i64 = sqlx::query_scalar("SELECT count(*) FROM invites")
        .fetch_one(&pool)
        .await?;

    assert_eq!(
        unavailable
            .service
            .create_invite(
                owner_id,
                group.id,
                InviteCreateInput {
                    expires_at: None,
                    max_uses: None,
                },
                "limiter-down:issue",
            )
            .await,
        Err(GroupsError::RateLimitUnavailable)
    );
    assert_eq!(
        unavailable
            .service
            .redeem_invite(joiner_id, invite.code, "limiter-down:redeem")
            .await,
        Err(GroupsError::RateLimitUnavailable)
    );
    let invite_count_after: i64 = sqlx::query_scalar("SELECT count(*) FROM invites")
        .fetch_one(&pool)
        .await?;
    assert_eq!(invite_count_after, invite_count_before);
    assert_eq!(membership_count(&pool, group.id, joiner_id).await?, 0);
    assert_eq!(invite_use_count(&pool, invite.id).await?, 0);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn soft_deleted_group_rejects_invite_issue_and_redeem_as_not_found() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "삭제 초대 소유자").await?;
    let joiner_id = insert_user(&pool, "삭제 초대 가입자").await?;
    let group = create_group(&fixture, owner_id).await?;
    let invite = create_invite(&fixture, owner_id, group.id, Some(1)).await?;
    fixture.service.delete_group(owner_id, group.id).await?;

    assert_eq!(
        fixture
            .service
            .create_invite(
                owner_id,
                group.id,
                InviteCreateInput {
                    expires_at: None,
                    max_uses: None,
                },
                "deleted-group:issue",
            )
            .await,
        Err(GroupsError::GroupNotFound)
    );
    assert_eq!(
        fixture
            .service
            .redeem_invite(joiner_id, invite.code, "deleted-group:redeem")
            .await,
        Err(GroupsError::GroupNotFound)
    );
    assert_eq!(membership_count(&pool, group.id, joiner_id).await?, 0);
    assert_eq!(invite_use_count(&pool, invite.id).await?, 0);

    pool.close().await;
    database.dispose().await
}

async fn membership_count(
    pool: &sqlx::PgPool,
    group_id: uuid::Uuid,
    user_id: uuid::Uuid,
) -> TestResult<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM memberships WHERE group_id = $1 AND user_id = $2")
            .bind(group_id)
            .bind(user_id)
            .fetch_one(pool)
            .await?,
    )
}

async fn invite_use_count(pool: &sqlx::PgPool, invite_id: uuid::Uuid) -> TestResult<i32> {
    Ok(
        sqlx::query_scalar("SELECT used_count FROM invites WHERE id = $1")
            .bind(invite_id)
            .fetch_one(pool)
            .await?,
    )
}
