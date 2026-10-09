//! R8 masking through the real send path (message send and topic announcement) and the
//! blocker push suppression of the message fan-out.

use std::sync::Arc;

use jamye_server::{
    adapters::postgres::{
        chatrooms::PostgresChatroomsRepository, media::PostgresMediaRepository,
        messaging::PostgresMessagingRepository, notifications::PostgresNotificationsRepository,
        push::PostgresPushRepository, topics::PostgresTopicsRepository,
        transactions::SqlxTransactionManager,
    },
    application::{
        chatrooms::ChatroomsService,
        messaging::{MessagingService, SendMessageInput, SendMessageOutcome},
        topics::{TopicCreateInput, TopicsDependencies, TopicsService},
        transactions::{TransactionCompositionDependencies, TransactionCompositions},
    },
    domain::moderation::ContentFilter,
    ports::push::PushPreviewSource,
};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    TestResult,
    postgres_support::TestDatabase,
    support::{MASKED_TERM, insert_group, insert_installation, insert_user},
};

struct Stack {
    database: TestDatabase,
    pool: PgPool,
    compositions: TransactionCompositions,
}

impl Stack {
    async fn new(filter: ContentFilter) -> TestResult<Self> {
        let database = TestDatabase::migrated().await?;
        let pool = database.pool()?;
        let transactions = Arc::new(SqlxTransactionManager::new(pool.clone()));
        let messaging = Arc::new(
            MessagingService::new(
                transactions.clone(),
                Arc::new(PostgresMessagingRepository::new(pool.clone())),
            )
            .with_content_filter(filter.clone()),
        );
        let topics = Arc::new(
            TopicsService::new(TopicsDependencies {
                transactions: transactions.clone(),
                repository: Arc::new(PostgresTopicsRepository::new(pool.clone())),
            })
            .with_content_filter(filter),
        );
        let chatrooms = Arc::new(ChatroomsService::new(
            transactions.clone(),
            Arc::new(PostgresChatroomsRepository::new(pool.clone())),
        ));
        let compositions = TransactionCompositions::new(TransactionCompositionDependencies {
            transactions: transactions.clone(),
            messaging,
            media: Arc::new(PostgresMediaRepository::new(pool.clone())),
            topics,
            chatrooms,
            notifications: Arc::new(PostgresNotificationsRepository::new(pool.clone())),
        });
        Ok(Self {
            database,
            pool,
            compositions,
        })
    }

    async fn finish(self, result: TestResult) -> TestResult {
        self.pool.close().await;
        let cleanup = self.database.dispose().await;
        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(failure), Ok(())) => Err(failure),
            (Ok(()), Err(cleanup_failure)) => Err(cleanup_failure),
            (Err(failure), Err(cleanup_failure)) => Err(format!(
                "message test failed: {failure}; database cleanup also failed: {cleanup_failure}"
            )
            .into()),
        }
    }

    async fn send(
        &self,
        sender: Uuid,
        chatroom_id: Uuid,
        client_msg_id: Uuid,
        body: &str,
    ) -> TestResult<(Uuid, Option<String>, bool)> {
        let outcome = self
            .compositions
            .send_message_http(
                sender,
                SendMessageInput {
                    chatroom_id,
                    client_msg_id,
                    body: Some(body.to_owned()),
                    media_upload_ids: Vec::new(),
                    idempotency_key: None,
                },
            )
            .await?;
        Ok(match outcome {
            SendMessageOutcome::Created(message) => (message.id, message.body, true),
            SendMessageOutcome::Existing(message) => (message.id, message.body, false),
        })
    }
}

#[tokio::test]
async fn message_bodies_are_masked_before_storage_event_outbox_and_push_preview() -> TestResult {
    let stack = Stack::new(ContentFilter::from_terms([MASKED_TERM])?).await?;
    let result: TestResult = async {
        let sender = insert_user(&stack.pool, "sender").await?;
        let reader = insert_user(&stack.pool, "reader").await?;
        let group = insert_group(&stack.pool, &[sender, reader]).await?;
        let client_msg_id = Uuid::new_v4();
        let raw = format!("a {MASKED_TERM} b {}", MASKED_TERM.to_uppercase());

        let (message_id, body, created) = stack
            .send(sender, group.chatroom_id, client_msg_id, &raw)
            .await?;
        assert!(created);
        assert_eq!(body.as_deref(), Some("a *** b ***"));

        let stored =
            sqlx::query_scalar::<_, Option<String>>("SELECT body FROM messages WHERE id = $1")
                .bind(message_id)
                .fetch_one(&stack.pool)
                .await?;
        assert_eq!(stored.as_deref(), Some("a *** b ***"));
        let event_body = sqlx::query_scalar::<_, Option<String>>(
            "SELECT payload ->> 'body' FROM conversation_events \
             WHERE conversation_id = $1 AND event_type = 'message.created'",
        )
        .bind(group.chatroom_id)
        .fetch_one(&stack.pool)
        .await?;
        assert_eq!(event_body.as_deref(), Some("a *** b ***"));
        for sql in [
            "SELECT count(*) FROM conversation_events WHERE payload::text ILIKE $1",
            "SELECT count(*) FROM outbox_events WHERE payload::text ILIKE $1",
        ] {
            let leaked = sqlx::query_scalar::<_, i64>(sql)
                .bind(format!("%{MASKED_TERM}%"))
                .fetch_one(&stack.pool)
                .await?;
            assert_eq!(leaked, 0, "an event table still carries the unmasked term");
        }
        let preview = PostgresPushRepository::new(stack.pool.clone())
            .load_message_body(message_id)
            .await?;
        assert_eq!(preview.as_deref(), Some("a *** b ***"));

        // An idempotent retry carries the raw body again and still converges.
        let (retry_id, retry_body, created) = stack
            .send(sender, group.chatroom_id, client_msg_id, &raw)
            .await?;
        assert!(!created);
        assert_eq!(retry_id, message_id);
        assert_eq!(retry_body.as_deref(), Some("a *** b ***"));
        Ok(())
    }
    .await;
    stack.finish(result).await
}

#[tokio::test]
async fn bodies_without_a_listed_term_are_stored_byte_for_byte() -> TestResult {
    let stack = Stack::new(ContentFilter::from_terms([MASKED_TERM])?).await?;
    let result: TestResult = async {
        let sender = insert_user(&stack.pool, "sender").await?;
        let group = insert_group(&stack.pool, &[sender]).await?;
        for body in [
            "  plain   body\n\twith  spacing \u{200b} and 한글 🙂  ",
            "ordinary profanity: damn, shit, 씨발",
            "secret word split, secret-word, secretwor",
        ] {
            let (message_id, returned, _) = stack
                .send(sender, group.chatroom_id, Uuid::new_v4(), body)
                .await?;
            assert_eq!(returned.as_deref(), Some(body));
            let stored =
                sqlx::query_scalar::<_, Option<String>>("SELECT body FROM messages WHERE id = $1")
                    .bind(message_id)
                    .fetch_one(&stack.pool)
                    .await?;
            assert_eq!(stored.as_deref(), Some(body));
        }
        Ok(())
    }
    .await;
    stack.finish(result).await
}

#[tokio::test]
async fn topic_announcements_are_masked_but_the_topic_title_is_not_filtered() -> TestResult {
    let stack = Stack::new(ContentFilter::from_terms([MASKED_TERM])?).await?;
    let result: TestResult = async {
        let author = insert_user(&stack.pool, "author").await?;
        let member = insert_user(&stack.pool, "member").await?;
        let group = insert_group(&stack.pool, &[author, member]).await?;
        stack
            .compositions
            .create_topic_http(
                author,
                group.group_id,
                TopicCreateInput {
                    idempotency_key: Uuid::new_v4(),
                    title: format!("about {MASKED_TERM} today"),
                },
            )
            .await?;
        let announcement = sqlx::query_scalar::<_, Option<String>>(
            // The announcement is a message in the group's main chatroom that links to the topic.
            "SELECT message.body FROM messages message \
             JOIN chatrooms chatroom ON chatroom.id = message.chatroom_id \
             WHERE chatroom.group_id = $1 AND chatroom.type = 'main'",
        )
        .bind(group.group_id)
        .fetch_one(&stack.pool)
        .await?
        .ok_or("announcement body missing")?;
        assert!(announcement.contains("***"), "announcement was not masked");
        assert!(!announcement.contains(MASKED_TERM));
        // The topic.created event legitimately carries the unfiltered title, so only the
        // message events are checked for the announcement text.
        let leaked = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM conversation_events \
             WHERE event_type = 'message.created' AND payload::text ILIKE $1",
        )
        .bind(format!("%{MASKED_TERM}%"))
        .fetch_one(&stack.pool)
        .await?;
        assert_eq!(
            leaked, 0,
            "a message event still carries the unmasked announcement"
        );
        // Titles and other profile text are outside the filter scope (assumption A26).
        let title = sqlx::query_scalar::<_, String>("SELECT title FROM topics WHERE group_id = $1")
            .bind(group.group_id)
            .fetch_one(&stack.pool)
            .await?;
        assert!(title.contains(MASKED_TERM));
        Ok(())
    }
    .await;
    stack.finish(result).await
}

async fn recipients(pool: &PgPool, message_id: Uuid) -> TestResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar::<_, Uuid>(
        "SELECT recipient_user_id FROM push_delivery_intents \
         WHERE source_message_id = $1 ORDER BY recipient_user_id",
    )
    .bind(message_id)
    .fetch_all(pool)
    .await?)
}

#[tokio::test]
async fn the_message_fan_out_skips_recipients_who_blocked_the_sender() -> TestResult {
    let stack = Stack::new(ContentFilter::disabled()).await?;
    let result: TestResult = async {
        let sender = insert_user(&stack.pool, "sender").await?;
        let blocker = insert_user(&stack.pool, "blocker").await?;
        let other = insert_user(&stack.pool, "other").await?;
        let group = insert_group(&stack.pool, &[sender, blocker, other]).await?;
        for user in [sender, blocker, other] {
            insert_installation(&stack.pool, user).await?;
        }
        sqlx::query("INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1, $2)")
            .bind(blocker)
            .bind(sender)
            .execute(&stack.pool)
            .await?;

        // Group (main chatroom) message from the blocked sender: only `other` is pushed.
        let (message_id, _, _) = stack
            .send(sender, group.chatroom_id, Uuid::new_v4(), "hello group")
            .await?;
        assert_eq!(recipients(&stack.pool, message_id).await?, vec![other]);
        let notified =
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM notifications WHERE user_id = $1")
                .bind(blocker)
                .fetch_one(&stack.pool)
                .await?;
        assert_eq!(notified, 0, "the blocker still gets a notification row");

        // The block is one-directional: the blocker's own message reaches the blocked sender.
        let (own_message, _, _) = stack
            .send(blocker, group.chatroom_id, Uuid::new_v4(), "hello back")
            .await?;
        let mut expected = vec![sender, other];
        expected.sort();
        assert_eq!(recipients(&stack.pool, own_message).await?, expected);

        // Topic conversation: same rule.
        stack
            .compositions
            .create_topic_http(
                other,
                group.group_id,
                TopicCreateInput {
                    idempotency_key: Uuid::new_v4(),
                    title: "a topic".to_owned(),
                },
            )
            .await?;
        let topic_chatroom = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM chatrooms WHERE group_id = $1 AND type = 'topic'",
        )
        .bind(group.group_id)
        .fetch_one(&stack.pool)
        .await?;
        let (topic_message, _, _) = stack
            .send(sender, topic_chatroom, Uuid::new_v4(), "hello topic")
            .await?;
        assert_eq!(recipients(&stack.pool, topic_message).await?, vec![other]);

        // Everyone else is unchanged when nobody blocked the sender.
        let (third, _, _) = stack
            .send(other, topic_chatroom, Uuid::new_v4(), "hello again")
            .await?;
        let mut expected = vec![sender, blocker];
        expected.sort();
        assert_eq!(recipients(&stack.pool, third).await?, expected);
        Ok(())
    }
    .await;
    stack.finish(result).await
}
