use sqlx::PgPool;
use uuid::Uuid;

use super::TestResult;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OwnerFixture {
    pub user_id: Uuid,
    pub group_id: Uuid,
    pub membership_id: Uuid,
    pub chatroom_id: Uuid,
}

pub async fn insert_owner_fixture(pool: &PgPool) -> TestResult<OwnerFixture> {
    let fixture = OwnerFixture {
        user_id: Uuid::new_v4(),
        group_id: Uuid::new_v4(),
        membership_id: Uuid::new_v4(),
        chatroom_id: Uuid::new_v4(),
    };
    let mut transaction = pool.begin().await?;

    sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, $2)")
        .bind(fixture.user_id)
        .bind(format!("test-user-{}", fixture.user_id.simple()))
        .execute(&mut *transaction)
        .await?;
    sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, $2, $3)")
        .bind(fixture.group_id)
        .bind(format!("test-group-{}", fixture.group_id.simple()))
        .bind(fixture.user_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, 'owner')",
    )
    .bind(fixture.membership_id)
    .bind(fixture.group_id)
    .bind(fixture.user_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("INSERT INTO chatrooms (id, group_id, type) VALUES ($1, $2, 'main')")
        .bind(fixture.chatroom_id)
        .bind(fixture.group_id)
        .execute(&mut *transaction)
        .await?;

    transaction.commit().await?;
    Ok(fixture)
}
