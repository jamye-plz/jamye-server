use serde_json::Value;
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    application::realtime::membership_revocation::RealtimeControlIntent,
    ports::account_deletion::{AccountDeletionReport, AccountDeletionRepositoryError},
};

use super::database_error;

type GraceMembershipRow = (Option<Uuid>,);

pub(super) async fn start_grace_period(
    connection: &mut PgConnection,
    user_id: Uuid,
) -> Result<AccountDeletionReport, AccountDeletionRepositoryError> {
    let deleted_at = mark_user_deleted(connection, user_id).await?;
    let live_group_ids = mark_memberships_deleted(connection, user_id, deleted_at).await?;
    revoke_refresh_sessions(connection, user_id, deleted_at).await?;
    disable_push_installations(connection, user_id, deleted_at).await?;
    soft_delete_auth_identities(connection, user_id, deleted_at).await?;
    append_realtime_revocations(connection, user_id, &live_group_ids).await?;
    Ok(AccountDeletionReport {
        memberships_removed: 0,
        cleanup_intents_enqueued: 0,
    })
}

async fn mark_user_deleted(
    connection: &mut PgConnection,
    user_id: Uuid,
) -> Result<OffsetDateTime, AccountDeletionRepositoryError> {
    sqlx::query_scalar::<_, OffsetDateTime>(
        "WITH server_clock AS MATERIALIZED (SELECT clock_timestamp() AS at) \
         UPDATE users \
         SET deleted_at = (SELECT at FROM server_clock), \
             account_purge_claim_owner = NULL, \
             account_purge_claim_expires_at = NULL \
         WHERE id = $1 AND deleted_at IS NULL \
         RETURNING deleted_at",
    )
    .bind(user_id)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("account_grace_user_mark", error))?
    .ok_or(AccountDeletionRepositoryError::AccountNotFound)
}

async fn mark_memberships_deleted(
    connection: &mut PgConnection,
    user_id: Uuid,
    deleted_at: OffsetDateTime,
) -> Result<Vec<Uuid>, AccountDeletionRepositoryError> {
    let rows = sqlx::query_as::<_, GraceMembershipRow>(
        "UPDATE memberships membership \
         SET deleted_at = $2, account_deleted_at = $2 \
         FROM groups group_entry \
         WHERE membership.group_id = group_entry.id \
           AND membership.user_id = $1 \
           AND membership.deleted_at IS NULL \
         RETURNING CASE WHEN group_entry.deleted_at IS NULL THEN membership.group_id ELSE NULL END",
    )
    .bind(user_id)
    .bind(deleted_at)
    .fetch_all(connection)
    .await
    .map_err(|error| database_error("account_grace_membership_mark", error))?;
    Ok(rows.into_iter().filter_map(|row| row.0).collect())
}

async fn revoke_refresh_sessions(
    connection: &mut PgConnection,
    user_id: Uuid,
    deleted_at: OffsetDateTime,
) -> Result<(), AccountDeletionRepositoryError> {
    sqlx::query(
        "UPDATE refresh_sessions \
         SET revoked_at = COALESCE(revoked_at, GREATEST($2, created_at)), \
             deleted_at = COALESCE(deleted_at, $2) \
         WHERE user_id = $1 \
           AND deleted_at IS NULL",
    )
    .bind(user_id)
    .bind(deleted_at)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(|error| database_error("account_grace_refresh_revoke", error))
}

async fn disable_push_installations(
    connection: &mut PgConnection,
    user_id: Uuid,
    deleted_at: OffsetDateTime,
) -> Result<(), AccountDeletionRepositoryError> {
    sqlx::query(
        "UPDATE push_installations \
         SET disabled_at = COALESCE(disabled_at, $2), \
             deleted_at = COALESCE(deleted_at, $2), \
             installation_id = 'deleted-' || id::text, \
             token = 'deleted-' || id::text \
         WHERE user_id = $1 \
           AND deleted_at IS NULL",
    )
    .bind(user_id)
    .bind(deleted_at)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(|error| database_error("account_grace_push_disable", error))
}

async fn soft_delete_auth_identities(
    connection: &mut PgConnection,
    user_id: Uuid,
    deleted_at: OffsetDateTime,
) -> Result<(), AccountDeletionRepositoryError> {
    sqlx::query(
        "UPDATE auth_identities \
         SET deleted_at = COALESCE(deleted_at, $2) \
         WHERE user_id = $1 \
           AND deleted_at IS NULL",
    )
    .bind(user_id)
    .bind(deleted_at)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(|error| database_error("account_grace_identity_mark", error))
}

async fn append_realtime_revocations(
    connection: &mut PgConnection,
    user_id: Uuid,
    live_group_ids: &[Uuid],
) -> Result<(), AccountDeletionRepositoryError> {
    for group_id in live_group_ids {
        let intent = RealtimeControlIntent::membership_revoked(*group_id, user_id);
        let payload = serde_json::to_value(&intent)
            .map_err(|_| AccountDeletionRepositoryError::InvalidData)?;
        insert_control_intent(connection, &intent, payload).await?;
    }
    Ok(())
}

async fn insert_control_intent(
    connection: &mut PgConnection,
    intent: &RealtimeControlIntent,
    payload: Value,
) -> Result<(), AccountDeletionRepositoryError> {
    sqlx::query(
        "INSERT INTO outbox_events \
         (id, intent_type, event_type, event_version, aggregate_type, aggregate_id, \
          conversation_event_id, payload) \
         VALUES ($1, 'control', $2, $3, $4, $5, NULL, $6)",
    )
    .bind(intent.control_id())
    .bind(intent.event_type())
    .bind(intent.version())
    .bind(intent.aggregate_type())
    .bind(intent.aggregate_id())
    .bind(payload)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(|error| database_error("account_grace_realtime_control", error))
}
