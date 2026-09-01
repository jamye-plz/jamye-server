//! PostgreSQL media upload repository.

mod access;
mod finalize;
mod message_binding;

use sqlx::PgPool;
use time::OffsetDateTime;

use crate::{
    adapters::postgres::transactions::connection,
    domain::{media::MediaScope, messaging::MessageAttachment},
    ports::{
        media::{
            AuthoritativeMessageMediaCommand, AuthorizeMediaAccessQuery, BindMessageMediaCommand,
            CreateUploadIntentCommand, FinalizeUploadCommand, MediaAccessRecord, MediaRepository,
            MediaRepositoryError, MediaRepositoryFuture, PrepareUploadFinalizeQuery,
            UploadFinalizePreparation, UploadFinalizeRecord, UploadIntentRecord,
        },
        transactions::TransactionHandle,
    },
};

#[derive(Clone)]
pub struct PostgresMediaRepository {
    pool: PgPool,
}

impl PostgresMediaRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl MediaRepository for PostgresMediaRepository {
    fn create_upload_intent<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a CreateUploadIntentCommand,
    ) -> MediaRepositoryFuture<'a, UploadIntentRecord> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| MediaRepositoryError::InvalidData)?;
            let byte_size =
                i64::try_from(command.byte_size).map_err(|_| MediaRepositoryError::InvalidData)?;
            let ttl_seconds = i64::try_from(command.expires_in.as_secs())
                .map_err(|_| MediaRepositoryError::InvalidData)?;
            let scope = match command.scope {
                MediaScope::Chat => "chat",
                MediaScope::Topic => "topic",
            };

            let timestamps = sqlx::query_as::<_, (OffsetDateTime, OffsetDateTime)>(
                "WITH authorized_chat AS ( \
                     SELECT chatroom.group_id \
                     FROM chatrooms chatroom \
                     JOIN groups live_group \
                       ON live_group.id = chatroom.group_id \
                      AND live_group.deleted_at IS NULL \
                     JOIN memberships actor_membership \
                       ON actor_membership.group_id = chatroom.group_id \
                      AND actor_membership.user_id = $2 \
                     WHERE $4 = 'chat' AND chatroom.id = $5 \
                     FOR SHARE OF chatroom, live_group, actor_membership \
                 ), \
                 authorized_topic AS ( \
                     SELECT topic.group_id \
                     FROM topics topic \
                     JOIN groups live_group \
                       ON live_group.id = topic.group_id \
                      AND live_group.deleted_at IS NULL \
                     JOIN memberships actor_membership \
                       ON actor_membership.group_id = topic.group_id \
                      AND actor_membership.user_id = $2 \
                     WHERE $4 = 'topic' AND topic.id = $5 \
                     FOR SHARE OF topic, live_group, actor_membership \
                 ), \
                 authorized AS ( \
                     SELECT group_id FROM authorized_chat \
                     UNION ALL \
                     SELECT group_id FROM authorized_topic \
                 ), \
                 stamped AS ( \
                     SELECT clock_timestamp() AS created_at \
                 ) \
                 INSERT INTO media_uploads \
                     (id, user_id, object_key, scope, target_id, content_type, byte_size, \
                      filename, expires_at, created_at) \
                 SELECT $1, $2, $3, $4, $5, $6, $7, $8, \
                        stamped.created_at + make_interval(secs => $9::double precision), \
                        stamped.created_at \
                 FROM authorized \
                 CROSS JOIN stamped \
                 RETURNING expires_at, created_at",
            )
            .bind(command.id)
            .bind(command.user_id)
            .bind(&command.object_key)
            .bind(scope)
            .bind(command.target_id)
            .bind(&command.content_type)
            .bind(byte_size)
            .bind(&command.filename)
            .bind(ttl_seconds)
            .fetch_optional(connection)
            .await
            .map_err(|error| database_error("upload_intent_insert", error))?
            .ok_or(MediaRepositoryError::TargetNotAccessible)?;

            Ok(UploadIntentRecord {
                id: command.id,
                user_id: command.user_id,
                scope: command.scope,
                target_id: command.target_id,
                object_key: command.object_key.clone(),
                kind: command.kind,
                content_type: command.content_type.clone(),
                byte_size: command.byte_size,
                filename: command.filename.clone(),
                expires_at: timestamps.0,
                created_at: timestamps.1,
            })
        })
    }

    fn prepare_upload_finalize<'a>(
        &'a self,
        query: &'a PrepareUploadFinalizeQuery,
    ) -> MediaRepositoryFuture<'a, UploadFinalizePreparation> {
        Box::pin(finalize::prepare_upload_finalize(&self.pool, query))
    }

    fn finalize_upload<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a FinalizeUploadCommand,
    ) -> MediaRepositoryFuture<'a, UploadFinalizeRecord> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| MediaRepositoryError::InvalidData)?;
            finalize::finalize_upload(connection, command).await
        })
    }

    fn bind_message_media<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a BindMessageMediaCommand,
    ) -> MediaRepositoryFuture<'a, Vec<MessageAttachment>> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| MediaRepositoryError::InvalidData)?;
            message_binding::bind_message_media(connection, command).await
        })
    }

    fn bind_authoritative_message_media<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a AuthoritativeMessageMediaCommand,
    ) -> MediaRepositoryFuture<'a, Vec<MessageAttachment>> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| MediaRepositoryError::InvalidData)?;
            message_binding::bind_authoritative_message_media(connection, command).await
        })
    }

    fn authorize_media_access<'a>(
        &'a self,
        query: &'a AuthorizeMediaAccessQuery,
    ) -> MediaRepositoryFuture<'a, MediaAccessRecord> {
        Box::pin(access::authorize_media_access(&self.pool, query))
    }
}

fn database_error(operation: &'static str, error: sqlx::Error) -> MediaRepositoryError {
    if let sqlx::Error::Database(database) = &error
        && let Some(
            "media_uploads_scope_check"
            | "media_uploads_object_key_check"
            | "media_uploads_content_type_check"
            | "media_uploads_byte_size_check"
            | "media_uploads_filename_check"
            | "media_uploads_timestamp_check"
            | "uq_media_uploads_object_key",
        ) = database.constraint()
    {
        return MediaRepositoryError::InvalidData;
    }
    tracing::warn!(
        dependency = "postgres",
        failure_kind = "media",
        operation,
        "PostgreSQL media operation failed"
    );
    MediaRepositoryError::Unavailable
}
