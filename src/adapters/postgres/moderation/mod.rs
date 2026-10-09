//! PostgreSQL moderation repository: reports, user blocks, suspension, operator alerts.
//!
//! Every statement is parameterized. Reports reference users, messages and groups; account
//! purge deletes an account's blocks and reassigns retained report references to the
//! anonymous tombstone (see `account_deletion`).

mod admin;

use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    adapters::postgres::transactions::connection,
    ports::{
        moderation::{
            BlockRecord, BlockUserCommand, BlockedUser, EnqueueOperatorAlertCommand,
            InsertReportCommand, ModerationRepository, ModerationRepositoryError,
            ModerationRepositoryFuture, ReportRecord, ReportTarget, ReportedMedia, ReportedMessage,
            ResolveReportTargetQuery, ResolvedReportTarget, SuspendUserCommand, UnblockUserCommand,
        },
        transactions::TransactionHandle,
    },
};

const FOREIGN_KEY_VIOLATION: &str = "23503";
const DELETED_ACCOUNT_NICKNAME: &str = "탈퇴한 사용자";

#[derive(Clone)]
pub struct PostgresModerationRepository {
    pool: PgPool,
}

impl PostgresModerationRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl ModerationRepository for PostgresModerationRepository {
    fn block_user<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a BlockUserCommand,
    ) -> ModerationRepositoryFuture<'a, BlockRecord> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| ModerationRepositoryError::InvalidData)?;
            block_user(connection, command).await
        })
    }

    fn unblock_user<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a UnblockUserCommand,
    ) -> ModerationRepositoryFuture<'a, ()> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| ModerationRepositoryError::InvalidData)?;
            sqlx::query("DELETE FROM user_blocks WHERE blocker_id = $1 AND blocked_id = $2")
                .bind(command.blocker_id)
                .bind(command.blocked_id)
                .execute(connection)
                .await
                .map(|_| ())
                .map_err(|error| database_error("moderation_unblock", error))
        })
    }

    fn list_blocks(&self, blocker_id: Uuid) -> ModerationRepositoryFuture<'_, Vec<BlockedUser>> {
        Box::pin(async move {
            let rows = sqlx::query_as::<_, (Uuid, String, Option<String>, OffsetDateTime)>(
                "SELECT ub.blocked_id, \
                        CASE WHEN account.deleted_at IS NOT NULL THEN $2 ELSE account.nickname END, \
                        CASE WHEN account.deleted_at IS NOT NULL THEN NULL ELSE account.avatar_url END, \
                        ub.created_at \
                 FROM user_blocks ub \
                 JOIN users account ON account.id = ub.blocked_id \
                 WHERE ub.blocker_id = $1 \
                 ORDER BY ub.created_at DESC, ub.blocked_id",
            )
            .bind(blocker_id)
            .bind(DELETED_ACCOUNT_NICKNAME)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| database_error("moderation_list_blocks", error))?;
            Ok(rows
                .into_iter()
                .map(|(user_id, nickname, avatar_url, blocked_at)| BlockedUser {
                    user_id,
                    nickname,
                    avatar_url,
                    blocked_at,
                })
                .collect())
        })
    }

    fn resolve_report_target<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        query: &'a ResolveReportTargetQuery,
    ) -> ModerationRepositoryFuture<'a, ResolvedReportTarget> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| ModerationRepositoryError::InvalidData)?;
            match query.target {
                ReportTarget::Message(message_id) => {
                    resolve_message(connection, query.reporter_id, message_id).await
                }
                ReportTarget::User(user_id) => {
                    resolve_user(connection, query.reporter_id, user_id).await
                }
            }
        })
    }

    fn insert_report<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a InsertReportCommand,
    ) -> ModerationRepositoryFuture<'a, ReportRecord> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| ModerationRepositoryError::InvalidData)?;
            let (target_type, message_id, user_id) = match command.target {
                ReportTarget::Message(message_id) => ("message", Some(message_id), None),
                ReportTarget::User(user_id) => ("user", None, Some(user_id)),
            };
            let created_at = sqlx::query_scalar::<_, OffsetDateTime>(
                "INSERT INTO reports \
                     (id, reporter_id, target_type, target_message_id, target_user_id, \
                      target_group_id, reason, message_snapshot) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                 RETURNING created_at",
            )
            .bind(command.id)
            .bind(command.reporter_id)
            .bind(target_type)
            .bind(message_id)
            .bind(user_id)
            .bind(command.group_id)
            .bind(command.reason.as_str())
            .bind(&command.message_snapshot)
            .fetch_one(connection)
            .await
            .map_err(report_insert_error)?;
            Ok(ReportRecord {
                id: command.id,
                created_at,
            })
        })
    }

    fn enqueue_operator_alerts<'a>(
        &'a self,
        command: &'a EnqueueOperatorAlertCommand,
    ) -> ModerationRepositoryFuture<'a, u64> {
        Box::pin(async move {
            if command.operator_account_ids.is_empty() {
                return Ok(0);
            }
            // One content-free occurrence per active installation of each operator account.
            // The payload carries only the generic type and the report id.
            sqlx::query(
                "INSERT INTO push_delivery_intents \
                     (id, report_id, recipient_user_id, push_installation_id, \
                      installation_owner_epoch, message_preview_enabled_snapshot, payload) \
                 SELECT gen_random_uuid(), $1, installation.user_id, installation.id, \
                        installation.owner_epoch, false, \
                        jsonb_build_object('type', 'report', 'report_id', $1::UUID::TEXT) \
                 FROM push_installations installation \
                 JOIN users account \
                   ON account.id = installation.user_id AND account.deleted_at IS NULL \
                 WHERE installation.user_id = ANY($2::UUID[]) \
                   AND installation.disabled_at IS NULL \
                   AND installation.deleted_at IS NULL \
                 ON CONFLICT (report_id, push_installation_id) \
                     WHERE report_id IS NOT NULL AND deleted_at IS NULL \
                 DO NOTHING",
            )
            .bind(command.report_id)
            .bind(command.operator_account_ids.clone())
            .execute(&self.pool)
            .await
            .map(|result| result.rows_affected())
            .map_err(|error| database_error("moderation_operator_alert_enqueue", error))
        })
    }

    fn is_suspended(&self, user_id: Uuid) -> ModerationRepositoryFuture<'_, bool> {
        Box::pin(async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT suspended_at IS NOT NULL FROM users \
                 WHERE id = $1 AND deleted_at IS NULL",
            )
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map(|row| row.unwrap_or(false))
            .map_err(|error| database_error("moderation_suspension_lookup", error))
        })
    }

    fn suspend_user<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a SuspendUserCommand,
    ) -> ModerationRepositoryFuture<'a, ()> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| ModerationRepositoryError::InvalidData)?;
            let updated = sqlx::query(
                "UPDATE users \
                 SET suspended_at = COALESCE(suspended_at, clock_timestamp()), \
                     suspension_reason = COALESCE($2, suspension_reason) \
                 WHERE id = $1 AND deleted_at IS NULL",
            )
            .bind(command.user_id)
            .bind(command.reason.as_deref())
            .execute(&mut *connection)
            .await
            .map_err(|error| database_error("moderation_suspend", error))?;
            if updated.rows_affected() != 1 {
                return Err(ModerationRepositoryError::UserNotFound);
            }
            sqlx::query(
                "UPDATE refresh_sessions \
                 SET revoked_at = GREATEST(clock_timestamp(), created_at) \
                 WHERE user_id = $1 AND revoked_at IS NULL",
            )
            .bind(command.user_id)
            .execute(connection)
            .await
            .map(|_| ())
            .map_err(|error| database_error("moderation_suspend_revoke_sessions", error))
        })
    }

    fn unsuspend_user<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        user_id: Uuid,
    ) -> ModerationRepositoryFuture<'a, ()> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| ModerationRepositoryError::InvalidData)?;
            let updated = sqlx::query(
                "UPDATE users SET suspended_at = NULL, suspension_reason = NULL WHERE id = $1",
            )
            .bind(user_id)
            .execute(connection)
            .await
            .map_err(|error| database_error("moderation_unsuspend", error))?;
            if updated.rows_affected() == 1 {
                Ok(())
            } else {
                Err(ModerationRepositoryError::UserNotFound)
            }
        })
    }
}

async fn block_user(
    connection: &mut PgConnection,
    command: &BlockUserCommand,
) -> Result<BlockRecord, ModerationRepositoryError> {
    // The blocked account must be live; the share lock keeps a concurrent purge out.
    let live = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM users WHERE id = $1 AND deleted_at IS NULL FOR SHARE",
    )
    .bind(command.blocked_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| database_error("moderation_block_target", error))?;
    if live.is_none() {
        return Err(ModerationRepositoryError::UserNotFound);
    }
    sqlx::query(
        "INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1, $2) \
         ON CONFLICT (blocker_id, blocked_id) DO NOTHING",
    )
    .bind(command.blocker_id)
    .bind(command.blocked_id)
    .execute(&mut *connection)
    .await
    .map_err(|error| database_error("moderation_block_insert", error))?;
    let blocked_at = sqlx::query_scalar::<_, OffsetDateTime>(
        "SELECT created_at FROM user_blocks WHERE blocker_id = $1 AND blocked_id = $2",
    )
    .bind(command.blocker_id)
    .bind(command.blocked_id)
    .fetch_one(connection)
    .await
    .map_err(|error| database_error("moderation_block_read", error))?;
    Ok(BlockRecord {
        blocked_id: command.blocked_id,
        blocked_at,
    })
}

/// The reporter must be a live member of the message's live group and the message must be a
/// live user message; anything else is the same `TargetNotFound`.
async fn resolve_message(
    connection: &mut PgConnection,
    reporter_id: Uuid,
    message_id: Uuid,
) -> Result<ResolvedReportTarget, ModerationRepositoryError> {
    let (group_id, author_id, body) = sqlx::query_as::<_, (Uuid, Option<Uuid>, Option<String>)>(
        "SELECT chatroom.group_id, message.sender_id, message.body \
             FROM messages message \
             JOIN chatrooms chatroom \
               ON chatroom.id = message.chatroom_id AND chatroom.deleted_at IS NULL \
             JOIN groups live_group \
               ON live_group.id = chatroom.group_id AND live_group.deleted_at IS NULL \
             LEFT JOIN topics topic ON topic.id = chatroom.topic_id \
             JOIN memberships membership \
               ON membership.group_id = live_group.id \
              AND membership.user_id = $2 \
              AND membership.deleted_at IS NULL \
             JOIN users reporter \
               ON reporter.id = membership.user_id AND reporter.deleted_at IS NULL \
             WHERE message.id = $1 \
               AND message.deleted_at IS NULL \
               AND message.type = 'user' \
               AND (chatroom.topic_id IS NULL OR topic.deleted_at IS NULL)",
    )
    .bind(message_id)
    .bind(reporter_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| database_error("moderation_report_message", error))?
    .ok_or(ModerationRepositoryError::TargetNotFound)?;
    let media = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT media_upload_id, type, object_key FROM message_media \
         WHERE message_id = $1 ORDER BY position",
    )
    .bind(message_id)
    .fetch_all(connection)
    .await
    .map_err(|error| database_error("moderation_report_media", error))?
    .into_iter()
    .map(|(upload_id, kind, object_key)| ReportedMedia {
        upload_id,
        kind,
        object_key,
    })
    .collect();
    Ok(ResolvedReportTarget {
        group_id,
        message: Some(ReportedMessage {
            author_id,
            body,
            media,
        }),
    })
}

/// The reporter must share a live group with the live target user.
async fn resolve_user(
    connection: &mut PgConnection,
    reporter_id: Uuid,
    user_id: Uuid,
) -> Result<ResolvedReportTarget, ModerationRepositoryError> {
    let group_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT mine.group_id \
         FROM memberships mine \
         JOIN groups live_group \
           ON live_group.id = mine.group_id AND live_group.deleted_at IS NULL \
         JOIN users reporter \
           ON reporter.id = mine.user_id AND reporter.deleted_at IS NULL \
         JOIN memberships theirs \
           ON theirs.group_id = mine.group_id \
          AND theirs.user_id = $2 \
          AND theirs.deleted_at IS NULL \
         JOIN users target \
           ON target.id = theirs.user_id AND target.deleted_at IS NULL \
         WHERE mine.user_id = $1 AND mine.deleted_at IS NULL \
         ORDER BY mine.joined_at, mine.group_id \
         LIMIT 1",
    )
    .bind(reporter_id)
    .bind(user_id)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("moderation_report_user", error))?
    .ok_or(ModerationRepositoryError::TargetNotFound)?;
    Ok(ResolvedReportTarget {
        group_id,
        message: None,
    })
}

/// A foreign-key violation while inserting a report means a referenced row (the target, its
/// group, or the reporter) was purged between the access check and the insert (purge race).
/// The report target is gone from the reporter's point of view: `TargetNotFound`, not
/// `UserNotFound`.
fn report_insert_error(error: sqlx::Error) -> ModerationRepositoryError {
    if let sqlx::Error::Database(database) = &error
        && database.code().as_deref() == Some(FOREIGN_KEY_VIOLATION)
    {
        return ModerationRepositoryError::TargetNotFound;
    }
    database_error("moderation_report_insert", error)
}

fn database_error(operation: &'static str, error: sqlx::Error) -> ModerationRepositoryError {
    if let sqlx::Error::Database(database) = &error
        && database.code().as_deref() == Some(FOREIGN_KEY_VIOLATION)
    {
        return ModerationRepositoryError::UserNotFound;
    }
    tracing::warn!(
        dependency = "postgres",
        failure_kind = "moderation",
        operation,
        "PostgreSQL moderation operation failed"
    );
    ModerationRepositoryError::Unavailable
}
