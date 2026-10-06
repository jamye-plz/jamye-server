use std::{env, fs, path::Path};

use sqlx::{Connection, PgConnection};
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

const AVATAR_MIGRATION: &str = "migrations/0019_user_avatar_uploads.sql";

#[test]
fn migration_0019_is_forward_only_and_additive() -> TestResult {
    let sql = fs::read_to_string(AVATAR_MIGRATION)?;
    for required in [
        "-- migration: 0019_user_avatar_uploads",
        "-- prerequisite: 0018_add_apple_auth_identity_provider.sql",
        "-- reversibility: forward-only",
        "-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md",
        "-- rationale:",
        "-- lock impact:",
        "CREATE TABLE user_avatar_uploads",
        "object_key VARCHAR(512) NOT NULL",
        "CONSTRAINT uq_user_avatar_uploads_object_key UNIQUE (object_key)",
        "status IN ('pending', 'active', 'released')",
        "byte_size BETWEEN 1 AND 1048576",
        "ON user_avatar_uploads (user_id)\n    WHERE status = 'active'",
        "ON user_avatar_uploads (user_id)\n    WHERE status = 'pending'",
        "EXECUTE FUNCTION jamye_set_updated_at_if_changed()",
    ] {
        assert!(
            sql.contains(required),
            "0019 migration is missing: {required}"
        );
    }
    for forbidden in [
        "content_type",
        "confirmed_at",
        "ALTER TABLE",
        "DROP ",
        "INSERT INTO",
        "-- no-transaction",
    ] {
        assert!(
            !sql.contains(forbidden),
            "0019 migration is not additive: {forbidden}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn migration_0019_upgrades_0018_and_enforces_the_avatar_row_rules() -> TestResult {
    let database = TestDatabase::migrated_to(18).await?;
    let mut connection = database.connection().await?;
    let result: TestResult = async {
        assert!(!table_exists(&mut connection, "user_avatar_uploads").await?);
        let user_id = insert_user(&mut connection).await?;
        let other_user_id = insert_user(&mut connection).await?;

        let migrator = sqlx::migrate::Migrator::new(Path::new("migrations")).await?;
        migrator.run_to(19, &mut connection).await?;
        assert!(table_exists(&mut connection, "user_avatar_uploads").await?);
        let applied: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM _sqlx_migrations WHERE success AND version BETWEEN 1 AND 19",
        )
        .fetch_one(&mut connection)
        .await?;
        assert_eq!(applied, 19);

        let columns = sqlx::query_scalar::<_, String>(
            "SELECT column_name FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name = 'user_avatar_uploads' \
             ORDER BY column_name",
        )
        .fetch_all(&mut connection)
        .await?;
        assert_eq!(
            columns,
            [
                "byte_size",
                "created_at",
                "deleted_at",
                "expires_at",
                "id",
                "object_key",
                "released_at",
                "status",
                "updated_at",
                "user_id",
            ]
        );

        // One pending and one active row per user, any number of released rows.
        insert_row(&mut connection, user_id, "pending", 1_024).await?;
        assert!(
            insert_row(&mut connection, user_id, "pending", 1_024)
                .await
                .is_err(),
            "second pending row for one user was accepted"
        );
        insert_row(&mut connection, user_id, "active", 1_024).await?;
        assert!(
            insert_row(&mut connection, user_id, "active", 1_024)
                .await
                .is_err(),
            "second active row for one user was accepted"
        );
        insert_row(&mut connection, user_id, "released", 1_024).await?;
        insert_row(&mut connection, user_id, "released", 1_024).await?;
        insert_row(&mut connection, other_user_id, "pending", 1_024).await?;
        insert_row(&mut connection, other_user_id, "active", 1_024).await?;

        // Status and size CHECK constraints.
        assert!(
            insert_row(&mut connection, other_user_id, "confirmed", 1_024)
                .await
                .is_err(),
            "unknown status was accepted"
        );
        for invalid_size in [0, 1_048_577, -5] {
            assert!(
                insert_row(&mut connection, other_user_id, "released", invalid_size)
                    .await
                    .is_err(),
                "byte_size {invalid_size} was accepted"
            );
        }
        insert_row(&mut connection, other_user_id, "released", 1).await?;
        insert_row(&mut connection, other_user_id, "released", 1_048_576).await?;

        // object_key is unique across all rows.
        let shared_key = format!("avatar/{user_id}/shared");
        insert_row_with_key(&mut connection, user_id, "released", &shared_key).await?;
        assert!(
            insert_row_with_key(&mut connection, other_user_id, "released", &shared_key)
                .await
                .is_err(),
            "duplicate object_key was accepted"
        );

        // Existing audit convention: updated_at trigger advances on a real change.
        let row_id = insert_row(&mut connection, other_user_id, "released", 7).await?;
        let moved: bool = sqlx::query_scalar(
            "WITH previous AS (SELECT updated_at FROM user_avatar_uploads WHERE id = $1), \
                  changed AS ( \
                      UPDATE user_avatar_uploads SET byte_size = 8 WHERE id = $1 \
                      RETURNING updated_at \
                  ) \
             SELECT changed.updated_at > previous.updated_at FROM previous, changed",
        )
        .bind(row_id)
        .fetch_one(&mut connection)
        .await?;
        assert!(moved, "updated_at trigger did not advance");
        Ok(())
    }
    .await;

    connection.close().await?;
    database.dispose().await?;
    result
}

#[tokio::test]
async fn a_failed_0019_upgrade_rolls_back_the_avatar_relation() -> TestResult {
    let database = TestDatabase::migrated_to(18).await?;
    let fixture_dir = env::temp_dir().join(format!(
        "jamye-server-task-19-forced-migration-{}",
        Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&fixture_dir)?;
    for entry in fs::read_dir("migrations")? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| std::io::Error::other("migration filename is not UTF-8"))?;
        if name.ends_with(".sql") && name < "0019_user_avatar_uploads.sql" {
            fs::copy(entry.path(), fixture_dir.join(name))?;
        }
    }
    let avatar_sql = fs::read_to_string(AVATAR_MIGRATION)?;
    fs::write(
        fixture_dir.join("0019_user_avatar_uploads.sql"),
        format!("{avatar_sql}\n\nSELECT * FROM task_19_relation_that_must_not_exist;\n"),
    )?;

    let result: TestResult = async {
        let migrator = sqlx::migrate::Migrator::new(fixture_dir.as_path()).await?;
        let mut connection = database.connection().await?;
        assert!(migrator.run_to(19, &mut connection).await.is_err());
        assert!(
            !table_exists(&mut connection, "user_avatar_uploads").await?,
            "failed 0019 left the avatar table behind"
        );
        let applied: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM _sqlx_migrations WHERE success AND version = 19",
        )
        .fetch_one(&mut connection)
        .await?;
        assert_eq!(applied, 0);
        connection.close().await?;
        Ok(())
    }
    .await;

    fs::remove_dir_all(&fixture_dir)?;
    result?;
    database.dispose().await
}

async fn table_exists(connection: &mut PgConnection, table: &str) -> TestResult<bool> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS ( \
             SELECT 1 FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_name = $1 \
         )",
    )
    .bind(table)
    .fetch_one(connection)
    .await?)
}

async fn insert_user(connection: &mut PgConnection) -> TestResult<Uuid> {
    let user_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, 'avatar migration')")
        .bind(user_id)
        .execute(connection)
        .await?;
    Ok(user_id)
}

async fn insert_row(
    connection: &mut PgConnection,
    user_id: Uuid,
    status: &str,
    byte_size: i32,
) -> Result<Uuid, sqlx::Error> {
    let row_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO user_avatar_uploads \
             (id, user_id, object_key, byte_size, status, expires_at) \
         VALUES ($1, $2, $3, $4, $5, clock_timestamp() + interval '15 minutes')",
    )
    .bind(row_id)
    .bind(user_id)
    .bind(format!("avatar/{user_id}/{row_id}"))
    .bind(byte_size)
    .bind(status)
    .execute(connection)
    .await?;
    Ok(row_id)
}

async fn insert_row_with_key(
    connection: &mut PgConnection,
    user_id: Uuid,
    status: &str,
    object_key: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO user_avatar_uploads \
             (id, user_id, object_key, byte_size, status, expires_at) \
         VALUES ($1, $2, $3, 1024, $4, clock_timestamp() + interval '15 minutes')",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(object_key)
    .bind(status)
    .execute(connection)
    .await?;
    Ok(())
}
