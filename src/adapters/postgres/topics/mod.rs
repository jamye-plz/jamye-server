//! PostgreSQL topic lifecycle, cursor-unread, tag, and atomic-create adapter.

mod mutation;
mod query;

use sqlx::PgPool;

use crate::{
    adapters::postgres::transactions::connection,
    ports::{
        topics::{
            CreateTopicCommand, CreateTopicOutcome, GetTopicQuery, ListTopicDatesQuery,
            ListTopicMediaQuery, ListTopicTagsQuery, ListTopicsQuery, PatchTopicCommand,
            ReplaceTopicTagsCommand, TopicDatePage, TopicMediaPage, TopicNotificationContext,
            TopicPage, TopicRecord, TopicStatus, TopicTagPage, TopicsRepository,
            TopicsRepositoryError, TopicsRepositoryFuture,
        },
        transactions::TransactionHandle,
    },
};

#[derive(Clone)]
pub struct PostgresTopicsRepository {
    pool: PgPool,
}

impl PostgresTopicsRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl TopicsRepository for PostgresTopicsRepository {
    fn create_topic<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a CreateTopicCommand,
    ) -> TopicsRepositoryFuture<'a, CreateTopicOutcome> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| TopicsRepositoryError::InvalidData)?;
            mutation::create_topic(connection, command).await
        })
    }

    fn patch_topic<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a PatchTopicCommand,
    ) -> TopicsRepositoryFuture<'a, TopicRecord> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| TopicsRepositoryError::InvalidData)?;
            mutation::patch_topic(connection, command).await
        })
    }

    fn promote_enriched<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        topic_id: uuid::Uuid,
    ) -> TopicsRepositoryFuture<'a, TopicStatus> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| TopicsRepositoryError::InvalidData)?;
            mutation::promote_enriched(connection, topic_id).await
        })
    }

    fn replace_tags<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a ReplaceTopicTagsCommand,
    ) -> TopicsRepositoryFuture<'a, TopicTagPage> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| TopicsRepositoryError::InvalidData)?;
            mutation::replace_tags(connection, command).await
        })
    }

    fn list_topics(&self, query: ListTopicsQuery) -> TopicsRepositoryFuture<'_, TopicPage> {
        Box::pin(query::list_topics(&self.pool, query))
    }

    fn list_topic_dates(
        &self,
        query: ListTopicDatesQuery,
    ) -> TopicsRepositoryFuture<'_, TopicDatePage> {
        Box::pin(query::list_topic_dates(&self.pool, query))
    }

    fn get_topic(&self, query: GetTopicQuery) -> TopicsRepositoryFuture<'_, TopicRecord> {
        Box::pin(query::get_topic(&self.pool, query))
    }

    fn list_tags(&self, query: ListTopicTagsQuery) -> TopicsRepositoryFuture<'_, TopicTagPage> {
        Box::pin(query::list_tags(&self.pool, query))
    }

    fn list_media(&self, query: ListTopicMediaQuery) -> TopicsRepositoryFuture<'_, TopicMediaPage> {
        Box::pin(query::list_media(&self.pool, query))
    }

    fn notification_context<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        topic: &'a TopicRecord,
    ) -> TopicsRepositoryFuture<'a, TopicNotificationContext> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| TopicsRepositoryError::InvalidData)?;
            mutation::notification_context(connection, topic).await
        })
    }
}

pub(super) fn database_error(operation: &'static str, error: sqlx::Error) -> TopicsRepositoryError {
    if let sqlx::Error::Database(database) = &error {
        match database.constraint() {
            Some(
                "topics_request_fingerprint_check"
                | "topics_title_check"
                | "topics_body_check"
                | "topics_status_check"
                | "topics_timestamp_check"
                | "topic_media_type_check"
                | "topic_media_object_key_check"
                | "topic_media_width_check"
                | "topic_media_height_check"
                | "topic_media_byte_size_check"
                | "topic_tags_tag_check"
                | "topic_tags_source_check"
                | "topic_tags_confidence_check"
                | "uq_topic_tags_topic_tag"
                | "uq_topic_media_topic_object",
            ) => return TopicsRepositoryError::InvalidData,
            Some("uq_topics_author_idempotency") => {
                return TopicsRepositoryError::IdempotencyConflict;
            }
            _ => {}
        }
    }
    tracing::warn!(
        dependency = "postgres",
        failure_kind = "topics",
        operation,
        "PostgreSQL topic operation failed"
    );
    TopicsRepositoryError::Unavailable
}
