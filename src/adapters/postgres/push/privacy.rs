use sqlx::PgConnection;
use uuid::Uuid;

use crate::ports::push::{FenceGroupPushCommand, FenceMembershipPushCommand, PushRepositoryError};

use super::database_error;

const PRIVACY_REVOKED: &str = "privacy_revoked";

pub(super) async fn fence_membership_revocation(
    connection: &mut PgConnection,
    command: &FenceMembershipPushCommand,
) -> Result<(), PushRepositoryError> {
    let notification_ids =
        lock_member_notifications(connection, command.group_id, command.user_id).await?;
    fence_notification_occurrences(connection, &notification_ids).await
}

pub(super) async fn fence_group_deletion(
    connection: &mut PgConnection,
    command: &FenceGroupPushCommand,
) -> Result<(), PushRepositoryError> {
    lock_group_memberships(connection, command.group_id).await?;
    let notification_ids = lock_group_notifications(connection, command.group_id).await?;
    fence_notification_occurrences(connection, &notification_ids).await
}

async fn lock_group_memberships(
    connection: &mut PgConnection,
    group_id: Uuid,
) -> Result<(), PushRepositoryError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM memberships WHERE group_id = $1 ORDER BY id FOR UPDATE",
    )
    .bind(group_id)
    .fetch_all(connection)
    .await
    .map(|_| ())
    .map_err(|error| database_error("push_privacy_membership_lock", error))
}

async fn lock_member_notifications(
    connection: &mut PgConnection,
    group_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<Uuid>, PushRepositoryError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT notification.id \
         FROM notifications notification \
         JOIN chatrooms conversation ON conversation.id = notification.conversation_id \
         WHERE conversation.group_id = $1 AND notification.user_id = $2 \
         ORDER BY notification.id \
         FOR UPDATE OF notification",
    )
    .bind(group_id)
    .bind(user_id)
    .fetch_all(connection)
    .await
    .map_err(|error| database_error("push_privacy_member_notification_lock", error))
}

async fn lock_group_notifications(
    connection: &mut PgConnection,
    group_id: Uuid,
) -> Result<Vec<Uuid>, PushRepositoryError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT notification.id \
         FROM notifications notification \
         JOIN chatrooms conversation ON conversation.id = notification.conversation_id \
         WHERE conversation.group_id = $1 \
         ORDER BY notification.id \
         FOR UPDATE OF notification",
    )
    .bind(group_id)
    .fetch_all(connection)
    .await
    .map_err(|error| database_error("push_privacy_group_notification_lock", error))
}

async fn fence_notification_occurrences(
    connection: &mut PgConnection,
    notification_ids: &[Uuid],
) -> Result<(), PushRepositoryError> {
    if notification_ids.is_empty() {
        return Ok(());
    }
    lock_installations(connection, notification_ids).await?;
    let occurrence_ids = lock_live_occurrences(connection, notification_ids).await?;
    terminalize_occurrences(connection, &occurrence_ids).await
}

async fn lock_installations(
    connection: &mut PgConnection,
    notification_ids: &[Uuid],
) -> Result<(), PushRepositoryError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT installation.id \
         FROM push_installations installation \
         WHERE installation.id IN ( \
             SELECT occurrence.push_installation_id \
             FROM push_delivery_intents occurrence \
             WHERE occurrence.notification_id = ANY($1::UUID[]) \
         ) \
         ORDER BY installation.id \
         FOR UPDATE",
    )
    .bind(notification_ids.to_vec())
    .fetch_all(connection)
    .await
    .map(|_| ())
    .map_err(|error| database_error("push_privacy_installation_lock", error))
}

async fn lock_live_occurrences(
    connection: &mut PgConnection,
    notification_ids: &[Uuid],
) -> Result<Vec<Uuid>, PushRepositoryError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM push_delivery_intents \
         WHERE notification_id = ANY($1::UUID[]) \
           AND status IN ('pending', 'claimed', 'retryable') \
         ORDER BY id \
         FOR UPDATE",
    )
    .bind(notification_ids.to_vec())
    .fetch_all(connection)
    .await
    .map_err(|error| database_error("push_privacy_occurrence_lock", error))
}

async fn terminalize_occurrences(
    connection: &mut PgConnection,
    occurrence_ids: &[Uuid],
) -> Result<(), PushRepositoryError> {
    if occurrence_ids.is_empty() {
        return Ok(());
    }
    let result = sqlx::query(
        "UPDATE push_delivery_intents \
         SET status = 'failed', claim_owner = NULL, lease_expires_at = NULL, \
             next_attempt_at = NULL, last_error_code = $2, failed_at = clock_timestamp() \
         WHERE id = ANY($1::UUID[]) \
           AND status IN ('pending', 'claimed', 'retryable')",
    )
    .bind(occurrence_ids.to_vec())
    .bind(PRIVACY_REVOKED)
    .execute(connection)
    .await
    .map_err(|error| database_error("push_privacy_occurrence_terminalize", error))?;
    let affected =
        usize::try_from(result.rows_affected()).map_err(|_| PushRepositoryError::InvalidData)?;
    if affected != occurrence_ids.len() {
        return Err(PushRepositoryError::InvalidData);
    }
    Ok(())
}
