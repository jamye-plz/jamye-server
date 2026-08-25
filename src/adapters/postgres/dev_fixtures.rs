//! PostgreSQL implementation of the guarded development fixture seed.

use sqlx::PgPool;

use crate::dev_fixtures::{DevFixtureStore, FixtureIds, FixtureSeedFuture, FixtureStoreError};

#[derive(Clone)]
pub struct PostgresDevFixtureStore {
    pool: PgPool,
}

impl PostgresDevFixtureStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl DevFixtureStore for PostgresDevFixtureStore {
    fn seed(&self, fixture: FixtureIds) -> FixtureSeedFuture<'_> {
        let pool = self.pool.clone();
        Box::pin(async move {
            seed_transaction(&pool, fixture).await.map_err(|_| {
                tracing::warn!(
                    failure_kind = "seed_transaction",
                    "development fixture seed failed"
                );
                FixtureStoreError
            })
        })
    }
}

async fn seed_transaction(pool: &PgPool, fixture: FixtureIds) -> Result<(), sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let nickname = format!("dev-user-{}", fixture.user_id.simple());
    let group_name = format!("dev-group-{}", fixture.group_id.simple());

    sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, $2)")
        .bind(fixture.user_id)
        .bind(nickname)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, $2, $3)")
        .bind(fixture.group_id)
        .bind(group_name)
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

    transaction.commit().await
}
