//! PostgreSQL avatar upload repository.
//!
//! Every mutation takes the owning `users` row lock first, so concurrent intents,
//! finalizations, and profile changes for one account are serialized and the partial
//! unique indexes (one pending, one active row per user) are never raced.

use sqlx::{PgConnection, PgExecutor, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    adapters::postgres::transactions::connection,
    ports::{
        auth::UserProfile,
        avatar::{
            ActivateAvatarCommand, ActiveAvatarRecord, AvatarFinalizePreparation,
            AvatarIntentRecord, AvatarRepository, AvatarRepositoryError, AvatarRepositoryFuture,
            CreateAvatarIntentCommand, PrepareAvatarFinalizeQuery,
        },
        transactions::TransactionHandle,
    },
};

type ProfileRow = (Uuid, String, String, Option<String>, OffsetDateTime);

const LIVE_PROFILE_SQL: &str = "SELECT account.id, identity.provider, account.nickname, \
            account.avatar_url, account.created_at \
     FROM users account \
     JOIN LATERAL ( \
         SELECT provider FROM auth_identities \
         WHERE user_id = account.id AND deleted_at IS NULL \
         ORDER BY created_at, id LIMIT 1 \
     ) identity ON TRUE \
     WHERE account.id = $1 AND account.deleted_at IS NULL";

#[derive(Clone)]
pub struct PostgresAvatarRepository {
    pool: PgPool,
}

impl PostgresAvatarRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl AvatarRepository for PostgresAvatarRepository {
    fn create_intent<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a CreateAvatarIntentCommand,
    ) -> AvatarRepositoryFuture<'a, AvatarIntentRecord> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| AvatarRepositoryError::InvalidData)?;
            create_intent(connection, command).await
        })
    }

    fn prepare_finalize<'a>(
        &'a self,
        query: &'a PrepareAvatarFinalizeQuery,
    ) -> AvatarRepositoryFuture<'a, AvatarFinalizePreparation> {
        Box::pin(prepare_finalize(&self.pool, query))
    }

    fn activate<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a ActivateAvatarCommand,
    ) -> AvatarRepositoryFuture<'a, UserProfile> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| AvatarRepositoryError::InvalidData)?;
            activate(connection, command).await
        })
    }

    fn find_active<'a>(
        &'a self,
        avatar_id: Uuid,
    ) -> AvatarRepositoryFuture<'a, Option<ActiveAvatarRecord>> {
        Box::pin(async move {
            let row = sqlx::query_as::<_, (String, i32)>(
                "SELECT object_key, byte_size FROM user_avatar_uploads \
                 WHERE id = $1 AND status = 'active' AND deleted_at IS NULL",
            )
            .bind(avatar_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| database_error("avatar_find_active", error))?;
            row.map(|(object_key, byte_size)| {
                Ok::<_, AvatarRepositoryError>(ActiveAvatarRecord {
                    object_key,
                    byte_size: u64::try_from(byte_size)
                        .map_err(|_| AvatarRepositoryError::InvalidData)?,
                })
            })
            .transpose()
        })
    }
}

async fn create_intent(
    connection: &mut PgConnection,
    command: &CreateAvatarIntentCommand,
) -> Result<AvatarIntentRecord, AvatarRepositoryError> {
    lock_live_user(connection, command.user_id).await?;
    let released = release_rows(connection, command.user_id, "pending")
        .await
        .map_err(|error| database_error("avatar_release_pending", error))?;
    enqueue_object_deletions(connection, &released)
        .await
        .map_err(|error| database_error("avatar_enqueue_pending", error))?;

    let byte_size =
        i32::try_from(command.byte_size).map_err(|_| AvatarRepositoryError::InvalidData)?;
    let ttl_seconds = i64::try_from(command.expires_in.as_secs())
        .map_err(|_| AvatarRepositoryError::InvalidData)?;
    let expires_at = sqlx::query_scalar::<_, OffsetDateTime>(
        "INSERT INTO user_avatar_uploads \
             (id, user_id, object_key, byte_size, status, expires_at) \
         VALUES ($1, $2, $3, $4, 'pending', \
                 clock_timestamp() + make_interval(secs => $5::double precision)) \
         RETURNING expires_at",
    )
    .bind(command.id)
    .bind(command.user_id)
    .bind(&command.object_key)
    .bind(byte_size)
    .bind(ttl_seconds)
    .fetch_one(connection)
    .await
    .map_err(|error| database_error("avatar_intent_insert", error))?;

    Ok(AvatarIntentRecord {
        id: command.id,
        user_id: command.user_id,
        object_key: command.object_key.clone(),
        byte_size: command.byte_size,
        expires_at,
    })
}

async fn prepare_finalize(
    pool: &PgPool,
    query: &PrepareAvatarFinalizeQuery,
) -> Result<AvatarFinalizePreparation, AvatarRepositoryError> {
    let (object_key, byte_size, status, expires_at, is_live) =
        sqlx::query_as::<_, (String, i32, String, OffsetDateTime, bool)>(
            "SELECT upload.object_key, upload.byte_size, upload.status, upload.expires_at, \
                    upload.expires_at > clock_timestamp() \
             FROM user_avatar_uploads upload \
             JOIN users account \
               ON account.id = upload.user_id AND account.deleted_at IS NULL \
             WHERE upload.id = $1 AND upload.user_id = $2 AND upload.deleted_at IS NULL",
        )
        .bind(query.upload_id)
        .bind(query.actor_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| database_error("avatar_finalize_prepare", error))?
        .ok_or(AvatarRepositoryError::UploadNotFound)?;

    match (status.as_str(), is_live) {
        ("pending", true) => Ok(AvatarFinalizePreparation::Pending(AvatarIntentRecord {
            id: query.upload_id,
            user_id: query.actor_id,
            object_key,
            byte_size: u64::try_from(byte_size).map_err(|_| AvatarRepositoryError::InvalidData)?,
            expires_at,
        })),
        ("active", _) => {
            let profile = load_profile(pool, query.actor_id).await?;
            Ok(AvatarFinalizePreparation::AlreadyActive(profile))
        }
        ("pending" | "released", _) => Err(AvatarRepositoryError::UploadNotPending),
        _ => Err(AvatarRepositoryError::InvalidData),
    }
}

async fn activate(
    connection: &mut PgConnection,
    command: &ActivateAvatarCommand,
) -> Result<UserProfile, AvatarRepositoryError> {
    lock_live_user(connection, command.actor_id).await?;
    let (status, is_live) = sqlx::query_as::<_, (String, bool)>(
        "SELECT status, expires_at > clock_timestamp() \
         FROM user_avatar_uploads \
         WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL \
         FOR UPDATE",
    )
    .bind(command.upload_id)
    .bind(command.actor_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| database_error("avatar_activate_lock", error))?
    .ok_or(AvatarRepositoryError::UploadNotFound)?;

    match (status.as_str(), is_live) {
        ("pending", true) => {}
        // A concurrent identical finalize already won; return the committed state.
        ("active", _) => return load_profile(&mut *connection, command.actor_id).await,
        ("pending" | "released", _) => return Err(AvatarRepositoryError::UploadNotPending),
        _ => return Err(AvatarRepositoryError::InvalidData),
    }

    let released = release_rows(connection, command.actor_id, "active")
        .await
        .map_err(|error| database_error("avatar_release_active", error))?;
    enqueue_object_deletions(connection, &released)
        .await
        .map_err(|error| database_error("avatar_enqueue_active", error))?;
    sqlx::query("UPDATE user_avatar_uploads SET status = 'active' WHERE id = $1")
        .bind(command.upload_id)
        .execute(&mut *connection)
        .await
        .map_err(|error| database_error("avatar_activate", error))?;

    let row = sqlx::query_as::<_, ProfileRow>(
        "WITH updated AS ( \
             UPDATE users SET avatar_url = $2 \
             WHERE id = $1 AND deleted_at IS NULL \
             RETURNING id, nickname, avatar_url, created_at \
         ) \
         SELECT updated.id, identity.provider, updated.nickname, \
                updated.avatar_url, updated.created_at \
         FROM updated \
         JOIN LATERAL ( \
             SELECT provider FROM auth_identities \
             WHERE user_id = updated.id AND deleted_at IS NULL \
             ORDER BY created_at, id LIMIT 1 \
         ) identity ON TRUE",
    )
    .bind(command.actor_id)
    .bind(&command.public_url)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("avatar_user_update", error))?
    .ok_or(AvatarRepositoryError::UploadNotFound)?;
    Ok(profile_from_row(row))
}

/// Release the user's active hosted avatar (if any) and queue its object for deletion.
///
/// Called by the profile update inside its transaction when `avatar_url` is cleared or set
/// to a different value. Setting the exact current URL again leaves the hosting row alone.
pub(crate) async fn release_active_for_profile_change(
    connection: &mut PgConnection,
    user_id: Uuid,
    new_avatar_url: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM users WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut *connection)
    .await?;
    let released = sqlx::query_scalar::<_, String>(
        "UPDATE user_avatar_uploads \
         SET status = 'released', released_at = clock_timestamp() \
         WHERE user_id = $1 AND status = 'active' \
           AND ($2::text IS NULL \
                OR (SELECT avatar_url FROM users WHERE id = $1) IS DISTINCT FROM $2::text) \
         RETURNING object_key",
    )
    .bind(user_id)
    .bind(new_avatar_url)
    .fetch_all(&mut *connection)
    .await?;
    enqueue_object_deletions(connection, &released).await
}

async fn lock_live_user(
    connection: &mut PgConnection,
    user_id: Uuid,
) -> Result<(), AvatarRepositoryError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM users WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("avatar_user_lock", error))?
    .map(|_| ())
    .ok_or(AvatarRepositoryError::UploadNotFound)
}

async fn release_rows(
    connection: &mut PgConnection,
    user_id: Uuid,
    status: &str,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar::<_, String>(
        "UPDATE user_avatar_uploads \
         SET status = 'released', released_at = clock_timestamp() \
         WHERE user_id = $1 AND status = $2 \
         RETURNING object_key",
    )
    .bind(user_id)
    .bind(status)
    .fetch_all(connection)
    .await
}

async fn enqueue_object_deletions(
    connection: &mut PgConnection,
    object_keys: &[String],
) -> Result<(), sqlx::Error> {
    for object_key in object_keys {
        sqlx::query(
            "INSERT INTO account_object_deletion_intents (id, object_key) \
             VALUES ($1, $2) \
             ON CONFLICT (object_key) DO NOTHING",
        )
        .bind(Uuid::new_v4())
        .bind(object_key)
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}

async fn load_profile<'e, E>(
    executor: E,
    user_id: Uuid,
) -> Result<UserProfile, AvatarRepositoryError>
where
    E: PgExecutor<'e>,
{
    sqlx::query_as::<_, ProfileRow>(LIVE_PROFILE_SQL)
        .bind(user_id)
        .fetch_optional(executor)
        .await
        .map_err(|error| database_error("avatar_profile_load", error))?
        .map(profile_from_row)
        .ok_or(AvatarRepositoryError::UploadNotFound)
}

fn profile_from_row((id, provider, nickname, avatar_url, created_at): ProfileRow) -> UserProfile {
    UserProfile {
        id,
        provider,
        nickname,
        avatar_url,
        created_at,
    }
}

fn database_error(operation: &'static str, error: sqlx::Error) -> AvatarRepositoryError {
    if let sqlx::Error::Database(database) = &error
        && let Some(
            "uq_user_avatar_uploads_object_key"
            | "user_avatar_uploads_byte_size_check"
            | "user_avatar_uploads_status_check",
        ) = database.constraint()
    {
        return AvatarRepositoryError::InvalidData;
    }
    tracing::warn!(
        dependency = "postgres",
        failure_kind = "avatar",
        operation,
        "PostgreSQL avatar operation failed"
    );
    AvatarRepositoryError::Unavailable
}
