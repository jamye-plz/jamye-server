use sqlx::PgConnection;
use uuid::Uuid;

use crate::ports::account_deletion::{
    AccountDeletionMembership, AccountDeletionPreparation, AccountDeletionRepositoryError,
};

use super::database_error;

type LockedGroupRow = (Uuid, Uuid, bool);
type LockedMembershipRow = (Uuid, Uuid, bool);

pub(super) async fn prepare_deletion(
    connection: &mut PgConnection,
    user_id: Uuid,
) -> Result<AccountDeletionPreparation, AccountDeletionRepositoryError> {
    lock_authentic_account(connection, user_id).await?;

    // Keep the global deletion lock order fixed. The later Task-6 and Task-9
    // calls re-lock subsets of these rows, which is safe because this caller
    // already owns the locks on the same PostgreSQL transaction.
    let groups = lock_affected_groups(connection, user_id).await?;
    if groups
        .iter()
        .any(|(_, owner_id, is_live)| *is_live && *owner_id == user_id)
    {
        return Err(AccountDeletionRepositoryError::GroupOwnershipTransferRequired);
    }

    let memberships = lock_target_memberships(connection, user_id).await?;
    lock_target_notifications(connection, user_id).await?;
    lock_referenced_installations(connection, user_id).await?;
    lock_referenced_occurrences(connection, user_id).await?;

    let memberships = memberships
        .into_iter()
        .filter(|(_, _, is_live)| *is_live)
        .map(|(membership_id, group_id, _)| AccountDeletionMembership {
            membership_id,
            group_id,
        })
        .collect();
    Ok(AccountDeletionPreparation { memberships })
}

pub(super) async fn lock_authentic_account(
    connection: &mut PgConnection,
    user_id: Uuid,
) -> Result<(), AccountDeletionRepositoryError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT account.id \
         FROM users account \
         WHERE account.id = $1 \
           AND NOT EXISTS ( \
               SELECT 1 FROM anonymous_author_tombstones tombstone \
               WHERE tombstone.user_id = account.id \
           ) \
         FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("account_deletion_account_lock", error))?
    .ok_or(AccountDeletionRepositoryError::AccountNotFound)
    .map(|_| ())
}

async fn lock_affected_groups(
    connection: &mut PgConnection,
    user_id: Uuid,
) -> Result<Vec<LockedGroupRow>, AccountDeletionRepositoryError> {
    sqlx::query_as::<_, LockedGroupRow>(
        "SELECT group_entry.id, group_entry.owner_id, group_entry.deleted_at IS NULL \
         FROM groups group_entry \
         WHERE group_entry.owner_id = $1 \
            OR EXISTS ( \
                SELECT 1 FROM memberships membership \
                WHERE membership.group_id = group_entry.id \
                  AND membership.user_id = $1 \
            ) \
         ORDER BY group_entry.id \
         FOR UPDATE",
    )
    .bind(user_id)
    .fetch_all(connection)
    .await
    .map_err(|error| database_error("account_deletion_group_lock", error))
}

async fn lock_target_memberships(
    connection: &mut PgConnection,
    user_id: Uuid,
) -> Result<Vec<LockedMembershipRow>, AccountDeletionRepositoryError> {
    sqlx::query_as::<_, LockedMembershipRow>(
        "SELECT membership.id, membership.group_id, group_entry.deleted_at IS NULL \
         FROM memberships membership \
         JOIN groups group_entry ON group_entry.id = membership.group_id \
         WHERE membership.user_id = $1 \
         ORDER BY membership.id \
         FOR UPDATE OF membership",
    )
    .bind(user_id)
    .fetch_all(connection)
    .await
    .map_err(|error| database_error("account_deletion_membership_lock", error))
}

async fn lock_target_notifications(
    connection: &mut PgConnection,
    user_id: Uuid,
) -> Result<(), AccountDeletionRepositoryError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT notification.id FROM notifications notification \
         WHERE notification.user_id = $1 \
            OR EXISTS ( \
                SELECT 1 \
                FROM conversation_events source_event \
                WHERE source_event.conversation_id = notification.conversation_id \
                  AND source_event.cursor = notification.source_cursor \
                  AND (source_event.payload ->> 'sender_id' = $1::UUID::TEXT \
                       OR source_event.payload ->> 'author_id' = $1::UUID::TEXT) \
            ) \
            OR EXISTS ( \
                SELECT 1 \
                FROM push_delivery_intents occurrence \
                JOIN push_installations installation \
                  ON installation.id = occurrence.push_installation_id \
                WHERE occurrence.notification_id = notification.id \
                  AND (occurrence.recipient_user_id = $1 \
                       OR installation.user_id = $1) \
            ) \
         ORDER BY notification.id \
         FOR UPDATE",
    )
    .bind(user_id)
    .fetch_all(connection)
    .await
    .map(|_| ())
    .map_err(|error| database_error("account_deletion_notification_lock", error))
}

async fn lock_referenced_installations(
    connection: &mut PgConnection,
    user_id: Uuid,
) -> Result<(), AccountDeletionRepositoryError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT installation.id FROM push_installations installation \
         WHERE installation.user_id = $1 \
            OR EXISTS ( \
                SELECT 1 \
                FROM push_delivery_intents occurrence \
                JOIN notifications notification ON notification.id = occurrence.notification_id \
                WHERE occurrence.push_installation_id = installation.id \
                  AND (occurrence.recipient_user_id = $1 \
                       OR notification.user_id = $1 \
                       OR EXISTS ( \
                           SELECT 1 \
                           FROM conversation_events source_event \
                           WHERE source_event.id = occurrence.source_event_id \
                             AND (source_event.payload ->> 'sender_id' = $1::UUID::TEXT \
                                  OR source_event.payload ->> 'author_id' = $1::UUID::TEXT) \
                       )) \
            ) \
         ORDER BY installation.id \
         FOR UPDATE",
    )
    .bind(user_id)
    .fetch_all(connection)
    .await
    .map(|_| ())
    .map_err(|error| database_error("account_deletion_installation_lock", error))
}

async fn lock_referenced_occurrences(
    connection: &mut PgConnection,
    user_id: Uuid,
) -> Result<(), AccountDeletionRepositoryError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT occurrence.id \
         FROM push_delivery_intents occurrence \
         JOIN notifications notification ON notification.id = occurrence.notification_id \
         JOIN push_installations installation ON installation.id = occurrence.push_installation_id \
         WHERE occurrence.recipient_user_id = $1 \
            OR notification.user_id = $1 \
            OR installation.user_id = $1 \
            OR EXISTS ( \
                SELECT 1 \
                FROM conversation_events source_event \
                WHERE source_event.id = occurrence.source_event_id \
                  AND (source_event.payload ->> 'sender_id' = $1::UUID::TEXT \
                       OR source_event.payload ->> 'author_id' = $1::UUID::TEXT) \
            ) \
         ORDER BY occurrence.id \
         FOR UPDATE OF occurrence",
    )
    .bind(user_id)
    .fetch_all(connection)
    .await
    .map(|_| ())
    .map_err(|error| database_error("account_deletion_occurrence_lock", error))
}
