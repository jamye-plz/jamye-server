use std::{
    env, fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use jamye_server::{
    adapters::redis::rate_limit::RedisRateLimiter,
    application::groups::{GroupsError, InviteCreateInput},
};

use crate::{
    TestResult,
    groups_helpers::{create_group, create_invite, harness_with_limiter, insert_user},
    postgres_support::TestDatabase,
    rate_limit::guarded_redis_url,
};

#[tokio::test]
#[ignore = "the task-6 Redis recovery card coordinates the guarded container lifecycle"]
async fn redis_stop_restart_preserves_invite_authority_and_recovers_the_same_limiter() -> TestResult
{
    let coordination_dir = recovery_coordination_dir()?;
    let redis_url = guarded_redis_url()?;
    let limiter = Arc::new(RedisRateLimiter::new(&redis_url)?);
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness_with_limiter(pool.clone(), limiter)?;
    let owner_id = insert_user(&pool, "Redis 복구 소유자").await?;
    let joiner_id = insert_user(&pool, "Redis 복구 가입자").await?;
    let group = create_group(&fixture, owner_id).await?;
    let invite = create_invite(&fixture, owner_id, group.id, Some(1)).await?;
    let authority_before = authority(&pool, group.id, invite.id, joiner_id).await?;
    assert_eq!(authority_before, (1, 0, 0));

    write_marker(&coordination_dir, "ready-to-stop")?;
    wait_for_marker(&coordination_dir, "redis-stopped").await?;
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
                "task6-recovery:issue",
            )
            .await,
        Err(GroupsError::RateLimitUnavailable)
    );
    assert_eq!(
        fixture
            .service
            .redeem_invite(joiner_id, invite.code.clone(), "task6-recovery:redeem")
            .await,
        Err(GroupsError::RateLimitUnavailable)
    );
    assert_eq!(
        authority(&pool, group.id, invite.id, joiner_id).await?,
        authority_before
    );

    write_marker(&coordination_dir, "ready-to-start")?;
    wait_for_marker(&coordination_dir, "redis-started").await?;
    let joined = fixture
        .service
        .redeem_invite(joiner_id, invite.code, "task6-recovery:redeem")
        .await?;
    assert!(joined.joined);
    assert_eq!(
        authority(&pool, group.id, invite.id, joiner_id).await?,
        (1, 1, 1)
    );

    pool.close().await;
    database.dispose().await
}

fn recovery_coordination_dir() -> TestResult<PathBuf> {
    let path = env::var("JAMYE_TASK6_RECOVERY_COORD_DIR").map_err(|_| {
        io::Error::other("JAMYE_TASK6_RECOVERY_COORD_DIR is required for the ignored recovery test")
    })?;
    let path = PathBuf::from(path);
    if !path.is_absolute() || !path.is_dir() {
        return Err(io::Error::other(
            "task-6 recovery coordination path must be an existing absolute directory",
        )
        .into());
    }
    Ok(path)
}

fn write_marker(directory: &Path, name: &str) -> TestResult {
    fs::write(directory.join(name), b"ready\n")?;
    Ok(())
}

async fn wait_for_marker(directory: &Path, name: &str) -> TestResult {
    let marker = directory.join(name);
    for _ in 0..600 {
        if marker.is_file() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(io::Error::other(format!(
        "timed out waiting for recovery marker {}",
        marker.display()
    ))
    .into())
}

async fn authority(
    pool: &sqlx::PgPool,
    group_id: uuid::Uuid,
    invite_id: uuid::Uuid,
    joiner_id: uuid::Uuid,
) -> TestResult<(i64, i32, i64)> {
    Ok(sqlx::query_as(
        "SELECT (SELECT count(*) FROM invites WHERE group_id = $1), \
                (SELECT used_count FROM invites WHERE id = $2), \
                (SELECT count(*) FROM memberships WHERE group_id = $1 AND user_id = $3)",
    )
    .bind(group_id)
    .bind(invite_id)
    .bind(joiner_id)
    .fetch_one(pool)
    .await?)
}
