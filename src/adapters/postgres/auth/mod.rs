//! PostgreSQL authentication identities, refresh families, and profiles.

use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    adapters::postgres::transactions::connection,
    ports::{
        auth::{
            AuthRepository, AuthRepositoryError, AuthRepositoryFuture, AvatarPatch,
            CredentialDigest, IssuedSession, NewProviderIdentity, NewRefreshSession,
            NewRotatedSession, ProfilePatch, RotationOutcome, UserProfile,
        },
        transactions::TransactionHandle,
    },
};

#[derive(Clone)]
pub struct PostgresAuthRepository {
    pool: PgPool,
}

impl PostgresAuthRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn create_session_record(
        &self,
        transaction: &mut dyn TransactionHandle,
        identity: &NewProviderIdentity,
        session: &NewRefreshSession,
    ) -> Result<IssuedSession, AuthRepositoryError> {
        let connection = connection(transaction).map_err(|_| AuthRepositoryError::InvalidData)?;
        let existing = sqlx::query_scalar::<_, Uuid>(
            "SELECT user_id FROM auth_identities WHERE provider = $1 AND provider_id = $2",
        )
        .bind(&identity.provider)
        .bind(&identity.provider_id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(|_| database_failure("identity_lookup"))?;
        let user_id = match existing {
            Some(user_id) => user_id,
            None => {
                let speculative_user_id = Uuid::new_v4();
                sqlx::query("INSERT INTO users (id, nickname, avatar_url) VALUES ($1, $2, $3)")
                    .bind(speculative_user_id)
                    .bind(&identity.nickname)
                    .bind(&identity.avatar_url)
                    .execute(&mut *connection)
                    .await
                    .map_err(|_| database_failure("user_insert"))?;
                let claimed = sqlx::query_scalar::<_, Uuid>(
                    "INSERT INTO auth_identities (id, user_id, provider, provider_id) \
                     VALUES ($1, $2, $3, $4) \
                     ON CONFLICT (provider, provider_id) DO NOTHING \
                     RETURNING user_id",
                )
                .bind(Uuid::new_v4())
                .bind(speculative_user_id)
                .bind(&identity.provider)
                .bind(&identity.provider_id)
                .fetch_optional(&mut *connection)
                .await
                .map_err(|_| database_failure("identity_insert"))?;
                match claimed {
                    Some(user_id) => user_id,
                    None => {
                        let canonical_user_id = sqlx::query_scalar::<_, Uuid>(
                            "SELECT user_id FROM auth_identities \
                             WHERE provider = $1 AND provider_id = $2",
                        )
                        .bind(&identity.provider)
                        .bind(&identity.provider_id)
                        .fetch_one(&mut *connection)
                        .await
                        .map_err(|_| database_failure("identity_convergence"))?;
                        sqlx::query("DELETE FROM users WHERE id = $1")
                            .bind(speculative_user_id)
                            .execute(&mut *connection)
                            .await
                            .map_err(|_| database_failure("speculative_user_cleanup"))?;
                        canonical_user_id
                    }
                }
            }
        };
        insert_refresh_session(
            &mut *connection,
            session.id,
            user_id,
            session.family_id,
            session.parent_session_id,
            &session.token_hash,
            session.expires_at,
        )
        .await?;
        Ok(IssuedSession {
            user_id,
            session_id: session.id,
        })
    }

    async fn rotate_session_record(
        &self,
        transaction: &mut dyn TransactionHandle,
        token_hash: &CredentialDigest,
        child: &NewRotatedSession,
        now: OffsetDateTime,
    ) -> Result<RotationOutcome, AuthRepositoryError> {
        let connection = connection(transaction).map_err(|_| AuthRepositoryError::InvalidData)?;
        let parent = sqlx::query_as::<_, (Uuid, Uuid, Uuid, OffsetDateTime, bool, bool)>(
            "SELECT id, user_id, family_id, expires_at, \
                    consumed_at IS NOT NULL, revoked_at IS NOT NULL \
             FROM refresh_sessions WHERE token_hash = $1 FOR UPDATE",
        )
        .bind(token_hash.as_bytes().as_slice())
        .fetch_optional(&mut *connection)
        .await
        .map_err(|_| database_failure("refresh_lock"))?;
        let Some((parent_id, user_id, family_id, expires_at, consumed, revoked)) = parent else {
            return Ok(RotationOutcome::Invalid);
        };
        if consumed {
            sqlx::query(
                "UPDATE refresh_sessions \
                 SET revoked_at = COALESCE(revoked_at, GREATEST($2, created_at)) \
                 WHERE family_id = $1",
            )
            .bind(family_id)
            .bind(now)
            .execute(&mut *connection)
            .await
            .map_err(|_| database_failure("refresh_family_revoke"))?;
            return Ok(RotationOutcome::Reused);
        }
        if revoked || expires_at <= now {
            return Ok(RotationOutcome::Invalid);
        }
        sqlx::query(
            "UPDATE refresh_sessions \
             SET consumed_at = GREATEST($2, created_at) WHERE id = $1",
        )
        .bind(parent_id)
        .bind(now)
        .execute(&mut *connection)
        .await
        .map_err(|_| database_failure("refresh_consume"))?;
        insert_refresh_session(
            &mut *connection,
            child.id,
            user_id,
            family_id,
            Some(parent_id),
            &child.token_hash,
            child.expires_at,
        )
        .await?;
        Ok(RotationOutcome::Rotated(IssuedSession {
            user_id,
            session_id: child.id,
        }))
    }

    async fn revoke_session_record(
        &self,
        transaction: &mut dyn TransactionHandle,
        session_id: Uuid,
        now: OffsetDateTime,
    ) -> Result<(), AuthRepositoryError> {
        let connection = connection(transaction).map_err(|_| AuthRepositoryError::InvalidData)?;
        sqlx::query(
            "UPDATE refresh_sessions \
             SET revoked_at = COALESCE(revoked_at, GREATEST($2, created_at)) WHERE id = $1",
        )
        .bind(session_id)
        .bind(now)
        .execute(connection)
        .await
        .map(|_| ())
        .map_err(|_| database_failure("refresh_revoke"))
    }

    async fn load_profile(
        &self,
        user_id: Uuid,
    ) -> Result<Option<UserProfile>, AuthRepositoryError> {
        sqlx::query_as::<_, (Uuid, String, String, Option<String>, OffsetDateTime)>(
            "SELECT u.id, identity.provider, u.nickname, u.avatar_url, u.created_at \
             FROM users u \
             JOIN LATERAL ( \
                 SELECT provider FROM auth_identities \
                 WHERE user_id = u.id ORDER BY created_at, id LIMIT 1 \
             ) identity ON TRUE \
             WHERE u.id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map(|row| row.map(profile_from_row))
        .map_err(|_| database_failure("profile_load"))
    }

    async fn update_profile_record(
        &self,
        transaction: &mut dyn TransactionHandle,
        user_id: Uuid,
        patch: &ProfilePatch,
    ) -> Result<Option<UserProfile>, AuthRepositoryError> {
        let connection = connection(transaction).map_err(|_| AuthRepositoryError::InvalidData)?;
        let (apply_avatar, avatar_url) = match &patch.avatar_url {
            AvatarPatch::Unchanged => (false, None),
            AvatarPatch::Set(value) => (true, Some(value.as_str())),
            AvatarPatch::Clear => (true, None),
        };
        sqlx::query_as::<_, (Uuid, String, String, Option<String>, OffsetDateTime)>(
            "WITH updated AS ( \
                 UPDATE users \
                 SET nickname = COALESCE($2, nickname), \
                     avatar_url = CASE WHEN $3 THEN $4 ELSE avatar_url END \
                 WHERE id = $1 \
                 RETURNING id, nickname, avatar_url, created_at \
             ) \
             SELECT updated.id, identity.provider, updated.nickname, \
                    updated.avatar_url, updated.created_at \
             FROM updated \
             JOIN LATERAL ( \
                 SELECT provider FROM auth_identities \
                 WHERE user_id = updated.id ORDER BY created_at, id LIMIT 1 \
             ) identity ON TRUE",
        )
        .bind(user_id)
        .bind(patch.nickname.as_deref())
        .bind(apply_avatar)
        .bind(avatar_url)
        .fetch_optional(connection)
        .await
        .map(|row| row.map(profile_from_row))
        .map_err(|_| database_failure("profile_update"))
    }
}

impl AuthRepository for PostgresAuthRepository {
    fn create_session<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        identity: &'a NewProviderIdentity,
        session: &'a NewRefreshSession,
    ) -> AuthRepositoryFuture<'a, IssuedSession> {
        Box::pin(self.create_session_record(transaction, identity, session))
    }

    fn rotate_session<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        token_hash: &'a CredentialDigest,
        child: &'a NewRotatedSession,
        now: OffsetDateTime,
    ) -> AuthRepositoryFuture<'a, RotationOutcome> {
        Box::pin(self.rotate_session_record(transaction, token_hash, child, now))
    }

    fn revoke_session<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        session_id: Uuid,
        now: OffsetDateTime,
    ) -> AuthRepositoryFuture<'a, ()> {
        Box::pin(self.revoke_session_record(transaction, session_id, now))
    }

    fn profile(&self, user_id: Uuid) -> AuthRepositoryFuture<'_, Option<UserProfile>> {
        Box::pin(self.load_profile(user_id))
    }

    fn update_profile<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        user_id: Uuid,
        patch: &'a ProfilePatch,
    ) -> AuthRepositoryFuture<'a, Option<UserProfile>> {
        Box::pin(self.update_profile_record(transaction, user_id, patch))
    }
}

async fn insert_refresh_session(
    connection: &mut sqlx::PgConnection,
    id: Uuid,
    user_id: Uuid,
    family_id: Uuid,
    parent_session_id: Option<Uuid>,
    token_hash: &CredentialDigest,
    expires_at: OffsetDateTime,
) -> Result<(), AuthRepositoryError> {
    sqlx::query(
        "INSERT INTO refresh_sessions \
         (id, user_id, family_id, parent_session_id, token_hash, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(user_id)
    .bind(family_id)
    .bind(parent_session_id)
    .bind(token_hash.as_bytes().as_slice())
    .bind(expires_at)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(|_| database_failure("refresh_insert"))
}

fn profile_from_row(row: (Uuid, String, String, Option<String>, OffsetDateTime)) -> UserProfile {
    UserProfile {
        id: row.0,
        provider: row.1,
        nickname: row.2,
        avatar_url: row.3,
        created_at: row.4,
    }
}

fn database_failure(operation: &'static str) -> AuthRepositoryError {
    tracing::warn!(
        dependency = "postgres",
        failure_kind = "auth",
        operation,
        "PostgreSQL authentication operation failed"
    );
    AuthRepositoryError::Unavailable
}
