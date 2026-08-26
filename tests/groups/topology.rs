use jamye_server::{
    application::groups::{GroupCreateInput, GroupsError, PageInput},
    ports::groups::GroupRole,
};
use uuid::Uuid;

use crate::{
    TestResult,
    groups_helpers::{create_group, harness, insert_member, insert_user},
    postgres_support::TestDatabase,
};

#[tokio::test]
async fn g1_commits_group_owner_and_exactly_one_canonical_main_chatroom() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "그룹 소유자").await?;

    let group = create_group(&fixture, owner_id).await?;
    assert_eq!(group.owner_id, owner_id);
    assert_eq!(group.max_members, 12);
    assert_eq!(group.member_count, 1);
    assert_ne!(group.main_chatroom_id, Uuid::nil());

    let owner_role: String =
        sqlx::query_scalar("SELECT role FROM memberships WHERE group_id = $1 AND user_id = $2")
            .bind(group.id)
            .bind(owner_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(owner_role, "owner");
    let main_ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM chatrooms WHERE group_id = $1 AND type = 'main'")
            .bind(group.id)
            .fetch_all(&pool)
            .await?;
    assert_eq!(main_ids, vec![group.main_chatroom_id]);

    let duplicate = sqlx::query(
        "INSERT INTO chatrooms (id, group_id, type, topic_id) VALUES ($1, $2, 'main', NULL)",
    )
    .bind(Uuid::new_v4())
    .bind(group.id)
    .execute(&pool)
    .await;
    let constraint = match duplicate {
        Err(sqlx::Error::Database(error)) => error.constraint().map(str::to_owned),
        other => return Err(format!("duplicate main result was {other:?}").into()),
    };
    assert_eq!(
        constraint.as_deref(),
        Some("ux_chatrooms_one_main_per_group")
    );
    let main_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM chatrooms WHERE group_id = $1 AND type = 'main'")
            .bind(group.id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(main_count, 1);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn failure_after_each_g1_insert_point_rolls_back_the_whole_topology() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "롤백 소유자").await?;

    sqlx::query(
        "CREATE FUNCTION task6_forced_g1_failure() RETURNS trigger \
         LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'task-6 forced G1 failure'; END $$",
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        "CREATE TRIGGER task6_fail_after_group BEFORE INSERT ON memberships \
         FOR EACH ROW EXECUTE FUNCTION task6_forced_g1_failure()",
    )
    .execute(&pool)
    .await?;
    assert_eq!(
        fixture
            .service
            .create_group(
                owner_id,
                GroupCreateInput {
                    name: "그룹 뒤 실패".to_owned(),
                },
            )
            .await,
        Err(GroupsError::DatabaseUnavailable)
    );
    sqlx::query("DROP TRIGGER task6_fail_after_group ON memberships")
        .execute(&pool)
        .await?;
    assert_topology_empty(&pool).await?;

    sqlx::query(
        "CREATE TRIGGER task6_fail_after_membership BEFORE INSERT ON chatrooms \
         FOR EACH ROW EXECUTE FUNCTION task6_forced_g1_failure()",
    )
    .execute(&pool)
    .await?;
    assert_eq!(
        fixture
            .service
            .create_group(
                owner_id,
                GroupCreateInput {
                    name: "멤버십 뒤 실패".to_owned(),
                },
            )
            .await,
        Err(GroupsError::DatabaseUnavailable)
    );
    sqlx::query("DROP TRIGGER task6_fail_after_membership ON chatrooms")
        .execute(&pool)
        .await?;
    assert_topology_empty(&pool).await?;

    sqlx::query(
        "CREATE CONSTRAINT TRIGGER task6_fail_after_chatroom AFTER INSERT ON chatrooms \
         DEFERRABLE INITIALLY DEFERRED FOR EACH ROW \
         EXECUTE FUNCTION task6_forced_g1_failure()",
    )
    .execute(&pool)
    .await?;
    assert_eq!(
        fixture
            .service
            .create_group(
                owner_id,
                GroupCreateInput {
                    name: "채팅방 뒤 실패".to_owned(),
                },
            )
            .await,
        Err(GroupsError::DatabaseUnavailable)
    );
    sqlx::query("DROP TRIGGER task6_fail_after_chatroom ON chatrooms")
        .execute(&pool)
        .await?;
    assert_topology_empty(&pool).await?;

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn ownership_transfer_is_atomic_and_owner_conflicts_are_stable() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "원 소유자").await?;
    let successor_id = insert_user(&pool, "새 소유자").await?;
    let group = create_group(&fixture, owner_id).await?;
    insert_member(&pool, group.id, successor_id, GroupRole::Member).await?;

    sqlx::query(
        "CREATE FUNCTION task6_forced_owner_failure() RETURNS trigger \
         LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'task-6 forced owner failure'; END $$",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TRIGGER task6_fail_owner_promotion BEFORE UPDATE OF role ON memberships \
         FOR EACH ROW WHEN (NEW.role = 'owner' AND OLD.role <> 'owner') \
         EXECUTE FUNCTION task6_forced_owner_failure()",
    )
    .execute(&pool)
    .await?;
    assert_eq!(
        fixture
            .service
            .set_member_role(owner_id, group.id, successor_id, GroupRole::Owner)
            .await,
        Err(GroupsError::DatabaseUnavailable)
    );
    assert_owner_state(&pool, group.id, owner_id, successor_id).await?;
    sqlx::query("DROP TRIGGER task6_fail_owner_promotion ON memberships")
        .execute(&pool)
        .await?;

    fixture
        .service
        .set_member_role(owner_id, group.id, successor_id, GroupRole::Owner)
        .await?;
    assert_owner_state(&pool, group.id, successor_id, owner_id).await?;
    assert_eq!(
        fixture
            .service
            .set_member_role(successor_id, group.id, successor_id, GroupRole::Owner)
            .await,
        Err(GroupsError::OwnerConflict)
    );
    assert_eq!(
        fixture
            .service
            .set_member_role(successor_id, group.id, successor_id, GroupRole::Member)
            .await,
        Err(GroupsError::OwnerConflict)
    );

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn soft_deleted_groups_are_hidden_from_every_membership_gated_query() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "삭제 소유자").await?;
    let group = create_group(&fixture, owner_id).await?;

    fixture.service.delete_group(owner_id, group.id).await?;
    assert_eq!(
        fixture.service.get_group(owner_id, group.id).await,
        Err(GroupsError::GroupNotFound)
    );
    assert_eq!(
        fixture
            .service
            .list_members(
                owner_id,
                group.id,
                PageInput {
                    after: None,
                    limit: None,
                },
            )
            .await,
        Err(GroupsError::GroupNotFound)
    );
    let page = fixture
        .service
        .list_groups(
            owner_id,
            PageInput {
                after: None,
                limit: None,
            },
        )
        .await?;
    assert!(page.items.is_empty());

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn member_pages_are_owner_first_strictly_forward_and_non_overlapping() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone())?;
    let owner_id = insert_user(&pool, "페이지 소유자").await?;
    let first_member = insert_user(&pool, "페이지 멤버 1").await?;
    let second_member = insert_user(&pool, "페이지 멤버 2").await?;
    let group = create_group(&fixture, owner_id).await?;
    insert_member(&pool, group.id, first_member, GroupRole::Member).await?;
    insert_member(&pool, group.id, second_member, GroupRole::Member).await?;

    let first = fixture
        .service
        .list_members(
            owner_id,
            group.id,
            PageInput {
                after: None,
                limit: Some(2),
            },
        )
        .await?;
    assert_eq!(first.items.len(), 2);
    assert_eq!(first.items[0].user_id, owner_id);
    assert_eq!(first.items[0].role, GroupRole::Owner);
    let cursor = first
        .next_cursor
        .ok_or_else(|| std::io::Error::other("first member page did not return a cursor"))?;
    let second = fixture
        .service
        .list_members(
            owner_id,
            group.id,
            PageInput {
                after: Some(cursor),
                limit: Some(2),
            },
        )
        .await?;
    assert_eq!(second.items.len(), 1);
    assert!(second.next_cursor.is_none());
    assert!(first.items.iter().all(|left| {
        second
            .items
            .iter()
            .all(|right| left.user_id != right.user_id)
    }));

    pool.close().await;
    database.dispose().await
}

async fn assert_topology_empty(pool: &sqlx::PgPool) -> TestResult {
    let counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM groups), \
                (SELECT count(*) FROM memberships), \
                (SELECT count(*) FROM chatrooms)",
    )
    .fetch_one(pool)
    .await?;
    assert_eq!(counts, (0, 0, 0));
    Ok(())
}

async fn assert_owner_state(
    pool: &sqlx::PgPool,
    group_id: Uuid,
    owner_id: Uuid,
    member_id: Uuid,
) -> TestResult {
    let stored_owner: Uuid = sqlx::query_scalar("SELECT owner_id FROM groups WHERE id = $1")
        .bind(group_id)
        .fetch_one(pool)
        .await?;
    assert_eq!(stored_owner, owner_id);
    let roles: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT user_id, role FROM memberships WHERE group_id = $1 ORDER BY user_id",
    )
    .bind(group_id)
    .fetch_all(pool)
    .await?;
    assert!(roles.contains(&(owner_id, "owner".to_owned())));
    assert!(roles.contains(&(member_id, "member".to_owned())));
    Ok(())
}
