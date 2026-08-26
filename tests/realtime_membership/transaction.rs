use jamye_server::application::realtime::membership_revocation::{
    ControlIntentError, MembershipRevocationError, RealtimeControlIntent,
};
use serde_json::Value;

use crate::{
    TestResult,
    helpers::{create_group, harness, insert_member, insert_user},
    postgres_support::TestDatabase,
};

#[tokio::test]
async fn remove_leave_and_group_delete_commit_exact_typed_control_intents() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "제어 소유자").await?;
    let removed_id = insert_user(&pool, "제거 멤버").await?;
    let leaving_id = insert_user(&pool, "탈퇴 멤버").await?;
    let group = create_group(&fixture, owner_id, "제어 그룹").await?;
    insert_member(&pool, group.id, removed_id).await?;
    insert_member(&pool, group.id, leaving_id).await?;

    let removed = fixture
        .revocations
        .remove_member(owner_id, group.id, removed_id)
        .await?;
    let left = fixture
        .revocations
        .remove_member(leaving_id, group.id, leaving_id)
        .await?;
    let deleted = fixture
        .revocations
        .delete_group(owner_id, group.id)
        .await?;

    assert!(matches!(
        removed,
        RealtimeControlIntent::MembershipRevoked { user_id, .. } if user_id == removed_id
    ));
    assert!(matches!(
        left,
        RealtimeControlIntent::MembershipRevoked { user_id, .. } if user_id == leaving_id
    ));
    assert!(matches!(deleted, RealtimeControlIntent::GroupDeleted { .. }));

    let membership_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM memberships WHERE group_id = $1 AND user_id = ANY($2::UUID[])",
    )
    .bind(group.id)
    .bind(vec![removed_id, leaving_id])
    .fetch_one(&pool)
    .await?;
    assert_eq!(membership_count, 0);
    let deleted_at_exists = sqlx::query_scalar::<_, bool>(
        "SELECT deleted_at IS NOT NULL FROM groups WHERE id = $1",
    )
    .bind(group.id)
    .fetch_one(&pool)
    .await?;
    assert!(deleted_at_exists);

    let rows = sqlx::query_as::<_, (String, i16, String, Value)>(
        "SELECT event_type, event_version, aggregate_type, payload \
         FROM outbox_events WHERE intent_type = 'control' ORDER BY created_at, id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows.iter()
            .filter(|row| row.0 == "membership.revoked" && row.2 == "membership")
            .count(),
        2
    );
    assert_eq!(
        rows.iter()
            .filter(|row| row.0 == "group.deleted" && row.2 == "group")
            .count(),
        1
    );
    for row in rows {
        assert_eq!(row.1, 1);
        assert_eq!(row.3["version"], 1);
        assert!(row.3.get("control_id").and_then(Value::as_str).is_some());
        assert!(row.3.get("group_id").and_then(Value::as_str).is_some());
    }

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn forced_control_insert_failure_rolls_back_the_real_task_6_mutation() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "롤백 소유자").await?;
    let member_id = insert_user(&pool, "롤백 멤버").await?;
    let group = create_group(&fixture, owner_id, "롤백 그룹").await?;
    insert_member(&pool, group.id, member_id).await?;

    sqlx::query(
        "CREATE FUNCTION reject_task_6c_control() RETURNS trigger LANGUAGE plpgsql AS \
         $task_6c$ BEGIN \
           IF NEW.intent_type = 'control' THEN \
             RAISE EXCEPTION 'task-6c injected control failure'; \
           END IF; \
           RETURN NEW; \
         END $task_6c$",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TRIGGER reject_task_6c_control BEFORE INSERT ON outbox_events \
         FOR EACH ROW EXECUTE FUNCTION reject_task_6c_control()",
    )
    .execute(&pool)
    .await?;

    assert_eq!(
        fixture
            .revocations
            .remove_member(owner_id, group.id, member_id)
            .await,
        Err(MembershipRevocationError::ControlIntent(
            ControlIntentError::Unavailable,
        ))
    );
    assert_eq!(
        fixture.revocations.delete_group(owner_id, group.id).await,
        Err(MembershipRevocationError::ControlIntent(
            ControlIntentError::Unavailable,
        ))
    );

    let membership_exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM memberships WHERE group_id = $1 AND user_id = $2)",
    )
    .bind(group.id)
    .bind(member_id)
    .fetch_one(&pool)
    .await?;
    assert!(membership_exists);
    let group_is_live = sqlx::query_scalar::<_, bool>(
        "SELECT deleted_at IS NULL FROM groups WHERE id = $1",
    )
    .bind(group.id)
    .fetch_one(&pool)
    .await?;
    assert!(group_is_live);
    let control_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM outbox_events WHERE intent_type = 'control'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(control_count, 0);

    pool.close().await;
    database.dispose().await
}
