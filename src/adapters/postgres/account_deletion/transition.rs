use sqlx::PgConnection;
use uuid::Uuid;

use crate::ports::account_deletion::{AccountDeletionReport, AccountDeletionRepositoryError};

use super::{
    database_error,
    payload_scrub::{scrub_retained_notification_profiles, scrub_retained_payloads},
    preparation::lock_authentic_account,
};

type LockedUploadRow = (Uuid, String);

pub(super) async fn finalize_deletion(
    connection: &mut PgConnection,
    user_id: Uuid,
) -> Result<AccountDeletionReport, AccountDeletionRepositoryError> {
    lock_authentic_account(connection, user_id).await?;
    let tombstone_user_id = Uuid::new_v4();
    create_tombstone(connection, tombstone_user_id).await?;

    delete_account_occurrences(connection, user_id).await?;
    reassign_retained_authorship(connection, user_id, tombstone_user_id).await?;
    scrub_retained_notification_profiles(connection, user_id).await?;
    scrub_retained_payloads(connection, user_id, tombstone_user_id).await?;
    reassign_archived_group_owners(connection, user_id, tombstone_user_id).await?;
    let archived_memberships_removed = delete_archived_memberships(connection, user_id).await?;
    ensure_no_live_memberships_remain(connection, user_id).await?;

    let cleanup_intents_enqueued = enqueue_and_delete_unbound_uploads(connection, user_id).await?;
    delete_private_account_rows(connection, user_id).await?;

    Ok(AccountDeletionReport {
        memberships_removed: archived_memberships_removed,
        cleanup_intents_enqueued,
    })
}

async fn create_tombstone(
    connection: &mut PgConnection,
    tombstone_user_id: Uuid,
) -> Result<(), AccountDeletionRepositoryError> {
    sqlx::query("INSERT INTO users (id, nickname, avatar_url) VALUES ($1, $2, NULL)")
        .bind(tombstone_user_id)
        .bind(crate::ports::account_deletion::ANONYMOUS_AUTHOR_NICKNAME)
        .execute(&mut *connection)
        .await
        .map_err(|error| database_error("account_deletion_tombstone_user_insert", error))?;
    sqlx::query("INSERT INTO anonymous_author_tombstones (user_id) VALUES ($1)")
        .bind(tombstone_user_id)
        .execute(connection)
        .await
        .map_err(|error| database_error("account_deletion_tombstone_insert", error))?;
    Ok(())
}

async fn delete_account_occurrences(
    connection: &mut PgConnection,
    user_id: Uuid,
) -> Result<(), AccountDeletionRepositoryError> {
    sqlx::query(
        "DELETE FROM push_delivery_intents occurrence \
         USING notifications notification, push_installations installation \
         WHERE occurrence.notification_id = notification.id \
           AND occurrence.push_installation_id = installation.id \
           AND (occurrence.recipient_user_id = $1 \
                OR notification.user_id = $1 \
                OR installation.user_id = $1 \
                OR EXISTS ( \
                    SELECT 1 \
                    FROM conversation_events source_event \
                    WHERE source_event.id = occurrence.source_event_id \
                      AND (source_event.payload ->> 'sender_id' = $1::UUID::TEXT \
                           OR source_event.payload ->> 'author_id' = $1::UUID::TEXT) \
                ))",
    )
    .bind(user_id)
    .execute(connection)
    .await
    .map(|_| ())
    .map_err(|error| database_error("account_deletion_occurrence_delete", error))
}

async fn reassign_retained_authorship(
    connection: &mut PgConnection,
    user_id: Uuid,
    tombstone_user_id: Uuid,
) -> Result<(), AccountDeletionRepositoryError> {
    for (operation, statement) in [
        (
            "account_deletion_message_author_reassign",
            "UPDATE messages SET sender_id = $2 WHERE sender_id = $1",
        ),
        (
            "account_deletion_topic_author_reassign",
            "UPDATE topics SET author_id = $2 WHERE author_id = $1",
        ),
        (
            "account_deletion_bound_upload_author_reassign",
            "UPDATE media_uploads SET user_id = $2 \
             WHERE user_id = $1 AND status = 'bound'",
        ),
    ] {
        sqlx::query(statement)
            .bind(user_id)
            .bind(tombstone_user_id)
            .execute(&mut *connection)
            .await
            .map_err(|error| database_error(operation, error))?;
    }
    Ok(())
}

async fn reassign_archived_group_owners(
    connection: &mut PgConnection,
    user_id: Uuid,
    tombstone_user_id: Uuid,
) -> Result<(), AccountDeletionRepositoryError> {
    sqlx::query(
        "UPDATE groups SET owner_id = $2 \
         WHERE owner_id = $1 AND deleted_at IS NOT NULL",
    )
    .bind(user_id)
    .bind(tombstone_user_id)
    .execute(connection)
    .await
    .map_err(|error| database_error("account_deletion_archived_group_owner_reassign", error))?;
    Ok(())
}

async fn delete_archived_memberships(
    connection: &mut PgConnection,
    user_id: Uuid,
) -> Result<u64, AccountDeletionRepositoryError> {
    let result = sqlx::query(
        "DELETE FROM memberships membership \
         USING groups group_entry \
         WHERE membership.user_id = $1 \
           AND membership.group_id = group_entry.id \
           AND group_entry.deleted_at IS NOT NULL",
    )
    .bind(user_id)
    .execute(connection)
    .await
    .map_err(|error| database_error("account_deletion_archived_membership_delete", error))?;
    Ok(result.rows_affected())
}

async fn ensure_no_live_memberships_remain(
    connection: &mut PgConnection,
    user_id: Uuid,
) -> Result<(), AccountDeletionRepositoryError> {
    let remaining =
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM memberships WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(connection)
            .await
            .map_err(|error| {
                database_error("account_deletion_remaining_membership_check", error)
            })?;
    if remaining == 0 {
        Ok(())
    } else {
        Err(AccountDeletionRepositoryError::InvalidData)
    }
}

async fn enqueue_and_delete_unbound_uploads(
    connection: &mut PgConnection,
    user_id: Uuid,
) -> Result<u64, AccountDeletionRepositoryError> {
    let uploads = sqlx::query_as::<_, LockedUploadRow>(
        "SELECT id, object_key FROM media_uploads \
         WHERE user_id = $1 \
           AND status IN ('pending', 'confirmed', 'expired') \
         ORDER BY id \
         FOR UPDATE",
    )
    .bind(user_id)
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| database_error("account_deletion_unbound_upload_lock", error))?;

    let mut cleanup_intents_enqueued = 0_u64;
    for (_, object_key) in &uploads {
        let result = sqlx::query(
            "INSERT INTO account_object_deletion_intents (id, object_key) \
             VALUES ($1, $2) \
             ON CONFLICT (object_key) DO NOTHING",
        )
        .bind(Uuid::new_v4())
        .bind(object_key)
        .execute(&mut *connection)
        .await
        .map_err(|error| database_error("account_deletion_cleanup_intent_insert", error))?;
        cleanup_intents_enqueued = cleanup_intents_enqueued
            .checked_add(result.rows_affected())
            .ok_or(AccountDeletionRepositoryError::InvalidData)?;
    }

    if uploads.is_empty() {
        return Ok(cleanup_intents_enqueued);
    }
    let upload_ids = uploads
        .into_iter()
        .map(|(upload_id, _)| upload_id)
        .collect::<Vec<_>>();
    let result = sqlx::query(
        "DELETE FROM media_uploads \
         WHERE id = ANY($1::UUID[]) \
           AND user_id = $2 \
           AND status IN ('pending', 'confirmed', 'expired')",
    )
    .bind(&upload_ids)
    .bind(user_id)
    .execute(connection)
    .await
    .map_err(|error| database_error("account_deletion_unbound_upload_delete", error))?;
    let deleted = usize::try_from(result.rows_affected())
        .map_err(|_| AccountDeletionRepositoryError::InvalidData)?;
    if deleted == upload_ids.len() {
        Ok(cleanup_intents_enqueued)
    } else {
        Err(AccountDeletionRepositoryError::InvalidData)
    }
}

async fn delete_private_account_rows(
    connection: &mut PgConnection,
    user_id: Uuid,
) -> Result<(), AccountDeletionRepositoryError> {
    for (operation, statement) in [
        (
            "account_deletion_notification_delete",
            "DELETE FROM notifications WHERE user_id = $1",
        ),
        (
            "account_deletion_installation_delete",
            "DELETE FROM push_installations WHERE user_id = $1",
        ),
        (
            "account_deletion_read_delete",
            "DELETE FROM chatroom_reads WHERE user_id = $1",
        ),
        (
            "account_deletion_invite_delete",
            "DELETE FROM invites WHERE created_by = $1",
        ),
        (
            "account_deletion_session_delete",
            "DELETE FROM refresh_sessions WHERE user_id = $1",
        ),
        (
            "account_deletion_identity_delete",
            "DELETE FROM auth_identities WHERE user_id = $1",
        ),
    ] {
        sqlx::query(statement)
            .bind(user_id)
            .execute(&mut *connection)
            .await
            .map_err(|error| database_error(operation, error))?;
    }

    let result = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(connection)
        .await
        .map_err(|error| database_error("account_deletion_user_delete", error))?;
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(AccountDeletionRepositoryError::AccountNotFound)
    }
}
