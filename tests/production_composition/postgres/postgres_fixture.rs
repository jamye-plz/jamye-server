impl PostgresFixture {
    pub(crate) async fn new(pool: PgPool) -> TestResult<Self> {
        let author_id = Uuid::new_v4();
        let recipient_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let topic_id = Uuid::new_v4();
        let topic_chatroom_id = Uuid::new_v4();
        let main_chatroom_id = Uuid::new_v4();
        for (id, nickname) in [
            (author_id, "Task-12 author"),
            (recipient_id, "Task-12 recipient"),
        ] {
            sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, $2)")
                .bind(id)
                .bind(nickname)
                .execute(&pool)
                .await?;
        }
        sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, $2, $3)")
            .bind(group_id)
            .bind("Task-12 group")
            .bind(author_id)
            .execute(&pool)
            .await?;
        for (user_id, role) in [(author_id, "owner"), (recipient_id, "member")] {
            sqlx::query(
                "INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, $4)",
            )
            .bind(Uuid::new_v4())
            .bind(group_id)
            .bind(user_id)
            .bind(role)
            .execute(&pool)
            .await?;
        }
        sqlx::query("INSERT INTO topics (id, group_id, author_id, idempotency_key, request_fingerprint, title) VALUES ($1, $2, $3, $4, $5, $6)").bind(topic_id).bind(group_id).bind(author_id).bind(Uuid::new_v4()).bind("a".repeat(64)).bind("Task-12 existing topic").execute(&pool).await?;
        for (id, kind, topic) in [
            (main_chatroom_id, "main", None),
            (topic_chatroom_id, "topic", Some(topic_id)),
        ] {
            sqlx::query(
                "INSERT INTO chatrooms (id, group_id, type, topic_id) VALUES ($1, $2, $3, $4)",
            )
            .bind(id)
            .bind(group_id)
            .bind(kind)
            .bind(topic)
            .execute(&pool)
            .await?;
        }
        let push_installation_id = Uuid::new_v4();
        sqlx::query("INSERT INTO push_installations (id, user_id, installation_id, platform, provider, token, environment, message_preview_enabled) VALUES ($1, $2, $3, 'ios', 'expo', $4, 'development', true)").bind(push_installation_id).bind(recipient_id).bind("task-12-recipient").bind("ExponentPushToken[task-12-recipient]").execute(&pool).await?;
        let seed_message_id = Uuid::new_v4();
        sqlx::query("INSERT INTO messages (id, chatroom_id, sender_id, client_msg_id, body, type) VALUES ($1, $2, $3, $4, $5, 'user')").bind(seed_message_id).bind(topic_chatroom_id).bind(author_id).bind(Uuid::new_v4()).bind("seed").execute(&pool).await?;
        let cursor = sqlx::query_scalar::<_, i64>("INSERT INTO conversation_events (id, conversation_id, event_type, event_version, payload) VALUES ($1, $2, 'message.created', 1, $3) RETURNING cursor").bind(Uuid::new_v4()).bind(topic_chatroom_id).bind(json!({"id": seed_message_id})).fetch_one(&pool).await?;
        let seeded_notification_id = Uuid::new_v4();
        sqlx::query("INSERT INTO notifications (id, user_id, topic_id, conversation_id, source_cursor, type, payload, dedup_key) VALUES ($1, $2, $3, $4, $5, 'chat_unread', '{}', $6)").bind(seeded_notification_id).bind(recipient_id).bind(topic_id).bind(topic_chatroom_id).bind(cursor).bind(format!("task-12-seed:{topic_id}")).execute(&pool).await?;
        let audio_upload_id = Uuid::new_v4();
        let audio = FinalizedObject {
            kind: MediaKind::Audio,
            content_type: "audio/ogg".to_owned(),
            byte_size: 8_192,
            duration_seconds: Some(38),
        };
        sqlx::query(
            "WITH stamped AS (SELECT clock_timestamp() AS now) \
             INSERT INTO media_uploads \
                (id, user_id, object_key, scope, target_id, content_type, byte_size, duration, filename, status, confirmed_at, expires_at, created_at) \
             SELECT $1, $2, $3, 'chat', $4, $5, $6, $7, $8, 'confirmed', stamped.now, stamped.now + INTERVAL '1 hour', stamped.now \
             FROM stamped",
        )
        .bind(audio_upload_id)
        .bind(author_id)
        .bind(format!("chat/{topic_chatroom_id}/{audio_upload_id}"))
        .bind(topic_chatroom_id)
        .bind(&audio.content_type)
        .bind(i64::try_from(audio.byte_size)?)
        .bind(audio.duration_seconds.map(i32::try_from).transpose()?)
        .bind("task-12-audio.ogg")
        .execute(&pool)
        .await?;
        let faults = Faults::default();
        let repositories = Arc::new(FaultingPostgresRepositories::new(
            pool.clone(),
            faults.clone(),
        ));
        let transactions = Arc::new(SqlxTransactionManager::new(pool.clone()));
        let compositions = TransactionCompositions::new(TransactionCompositionDependencies {
            transactions: transactions.clone(),
            messaging: Arc::new(MessagingService::new(
                transactions.clone(),
                repositories.clone(),
            )),
            media: repositories.clone(),
            topics: Arc::new(TopicsService::new(TopicsDependencies {
                transactions: transactions.clone(),
                repository: repositories.clone(),
            })),
            chatrooms: Arc::new(ChatroomsService::new(transactions, repositories.clone())),
            notifications: repositories,
        });
        Ok(Self {
            pool,
            compositions,
            faults,
            recipient_id,
            main_chatroom_id,
            topic_chatroom_id,
            audio_upload_id,
            push_installation_id,
            seeded_notification_id,
            send: SendMessageCompositionInput {
                message: SendMessageCommand {
                    chatroom_id: topic_chatroom_id,
                    sender_id: author_id,
                    client_msg_id: Uuid::new_v4(),
                    body: None,
                },
                group_id,
                topic_id,
                sender_display_name: "Task-12 author".to_owned(),
                media: vec![BindMessageMediaItem {
                    upload_id: audio_upload_id,
                    finalized: audio,
                }],
            },
            topic: CreateTopicCompositionInput {
                topic: CreateTopicCommand {
                    topic_id: Uuid::new_v4(),
                    topic_chatroom_id: Uuid::new_v4(),
                    topic_event_id: Uuid::new_v4(),
                    topic_outbox_id: Uuid::new_v4(),
                    author_read_marker_id: Uuid::new_v4(),
                    announcement_message_id: Uuid::new_v4(),
                    announcement_client_msg_id: Uuid::new_v4(),
                    announcement_event_id: Uuid::new_v4(),
                    announcement_outbox_id: Uuid::new_v4(),
                    group_id,
                    author_id,
                    idempotency_key: Uuid::new_v4(),
                    request_fingerprint: "b".repeat(64),
                    title: "Task-12 new topic".to_owned(),
                    announcement_body: "announcement".to_owned(),
                },
                author_display_name: "Task-12 author".to_owned(),
            },
            read: MarkConversationReadCompositionInput {
                read: MarkReadCommand {
                    marker_id: Uuid::new_v4(),
                    user_id: recipient_id,
                    chatroom_id: topic_chatroom_id,
                    cursor,
                },
            },
        })
    }
    fn arm(&self, boundary: Boundary) {
        self.faults.arm(boundary);
    }
    fn disarm(&self) {
        self.faults.clear();
    }
    fn reached(&self, boundary: Boundary) -> bool {
        self.faults.reached(boundary)
    }
    async fn run(
        &self,
        boundary: Boundary,
    ) -> Result<(), jamye_server::application::transactions::TransactionCompositionError> {
        match boundary {
            Boundary::SendMessage | Boundary::MessageMedia | Boundary::MessageNotification => {
                self.compositions.send_message(self.send.clone()).await
            }
            Boundary::CreateTopic | Boundary::TopicNotification => {
                self.compositions.create_topic(self.topic.clone()).await
            }
            Boundary::MarkRead | Boundary::NotificationClear => {
                self.compositions.mark_conversation_read(self.read).await
            }
        }
    }
    async fn persist_message(&self, client_msg_id: Uuid) -> TestResult<PersistedMessage> {
        let transactions = SqlxTransactionManager::new(self.pool.clone());
        let repository = PostgresMessagingRepository::new(self.pool.clone());
        let mut transaction = transactions.begin().await?;
        let result = repository
            .send(
                transaction.as_mut(),
                &SendMessageCommand {
                    chatroom_id: self.topic_chatroom_id,
                    sender_id: self.send.message.sender_id,
                    client_msg_id,
                    body: Some("Task-12 source event identity".to_owned()),
                },
            )
            .await;
        match result {
            Ok(outcome) => {
                transactions.commit(transaction).await?;
                Ok(outcome.into_persisted())
            }
            Err(error) => {
                transactions.rollback(transaction).await?;
                Err(io::Error::other(format!("message persistence failed: {error}")).into())
            }
        }
    }
}

#[derive(Clone)]
struct FaultingPostgresRepositories {
    messaging: PostgresMessagingRepository,
    media: PostgresMediaRepository,
    topics: PostgresTopicsRepository,
    chatrooms: PostgresChatroomsRepository,
    notifications: PostgresNotificationsRepository,
    faults: Faults,
}
impl FaultingPostgresRepositories {
    fn new(pool: PgPool, faults: Faults) -> Self {
        Self {
            messaging: PostgresMessagingRepository::new(pool.clone()),
            media: PostgresMediaRepository::new(pool.clone()),
            topics: PostgresTopicsRepository::new(pool.clone()),
            chatrooms: PostgresChatroomsRepository::new(pool.clone()),
            notifications: PostgresNotificationsRepository::new(pool),
            faults,
        }
    }
}

impl MessagingRepository for FaultingPostgresRepositories {
    fn send<'a>(
        &'a self,
        transaction: &'a mut dyn jamye_server::ports::transactions::TransactionHandle,
        command: &'a SendMessageCommand,
    ) -> MessagingFuture<'a, jamye_server::ports::messaging::PersistMessageOutcome> {
        Box::pin(async move {
            let value = self.messaging.send(transaction, command).await?;
            if self.faults.after(Boundary::SendMessage) {
                Err(MessagingRepositoryError::DatabaseUnavailable)
            } else {
                Ok(value)
            }
        })
    }
    fn events(
        &self,
        query: DeltaQuery,
    ) -> MessagingFuture<'_, jamye_server::domain::messaging::EventPage> {
        self.messaging.events(query)
    }
}
impl MediaRepository for FaultingPostgresRepositories {
    fn create_upload_intent<'a>(
        &'a self,
        transaction: &'a mut dyn jamye_server::ports::transactions::TransactionHandle,
        command: &'a CreateUploadIntentCommand,
    ) -> MediaRepositoryFuture<'a, UploadIntentRecord> {
        self.media.create_upload_intent(transaction, command)
    }
    fn prepare_upload_finalize<'a>(
        &'a self,
        query: &'a PrepareUploadFinalizeQuery,
    ) -> MediaRepositoryFuture<'a, UploadFinalizePreparation> {
        self.media.prepare_upload_finalize(query)
    }
    fn finalize_upload<'a>(
        &'a self,
        transaction: &'a mut dyn jamye_server::ports::transactions::TransactionHandle,
        command: &'a FinalizeUploadCommand,
    ) -> MediaRepositoryFuture<'a, UploadFinalizeRecord> {
        self.media.finalize_upload(transaction, command)
    }
    fn bind_message_media<'a>(
        &'a self,
        transaction: &'a mut dyn jamye_server::ports::transactions::TransactionHandle,
        command: &'a BindMessageMediaCommand,
    ) -> MediaRepositoryFuture<'a, Vec<jamye_server::domain::messaging::MessageAttachment>> {
        Box::pin(async move {
            let value = self.media.bind_message_media(transaction, command).await?;
            if self.faults.after(Boundary::MessageMedia) {
                Err(MediaRepositoryError::Unavailable)
            } else {
                Ok(value)
            }
        })
    }
    fn authorize_media_access<'a>(
        &'a self,
        query: &'a AuthorizeMediaAccessQuery,
    ) -> MediaRepositoryFuture<'a, MediaAccessRecord> {
        self.media.authorize_media_access(query)
    }
}
impl TopicsRepository for FaultingPostgresRepositories {
    fn create_topic<'a>(
        &'a self,
        transaction: &'a mut dyn jamye_server::ports::transactions::TransactionHandle,
        command: &'a CreateTopicCommand,
    ) -> TopicsRepositoryFuture<'a, CreateTopicOutcome> {
        Box::pin(async move {
            let value = self.topics.create_topic(transaction, command).await?;
            if self.faults.after(Boundary::CreateTopic) {
                Err(TopicsRepositoryError::Unavailable)
            } else {
                Ok(value)
            }
        })
    }
    fn patch_topic<'a>(
        &'a self,
        transaction: &'a mut dyn jamye_server::ports::transactions::TransactionHandle,
        command: &'a PatchTopicCommand,
    ) -> TopicsRepositoryFuture<'a, TopicRecord> {
        self.topics.patch_topic(transaction, command)
    }
    fn promote_enriched<'a>(
        &'a self,
        transaction: &'a mut dyn jamye_server::ports::transactions::TransactionHandle,
        topic_id: Uuid,
    ) -> TopicsRepositoryFuture<'a, TopicStatus> {
        self.topics.promote_enriched(transaction, topic_id)
    }
    fn replace_tags<'a>(
        &'a self,
        transaction: &'a mut dyn jamye_server::ports::transactions::TransactionHandle,
        command: &'a ReplaceTopicTagsCommand,
    ) -> TopicsRepositoryFuture<'a, TopicTagPage> {
        self.topics.replace_tags(transaction, command)
    }
    fn list_topics(&self, query: ListTopicsQuery) -> TopicsRepositoryFuture<'_, TopicPage> {
        self.topics.list_topics(query)
    }
    fn list_topic_dates(
        &self,
        query: ListTopicDatesQuery,
    ) -> TopicsRepositoryFuture<'_, TopicDatePage> {
        self.topics.list_topic_dates(query)
    }
    fn get_topic(&self, query: GetTopicQuery) -> TopicsRepositoryFuture<'_, TopicRecord> {
        self.topics.get_topic(query)
    }
    fn list_tags(&self, query: ListTopicTagsQuery) -> TopicsRepositoryFuture<'_, TopicTagPage> {
        self.topics.list_tags(query)
    }
    fn list_media(&self, query: ListTopicMediaQuery) -> TopicsRepositoryFuture<'_, TopicMediaPage> {
        self.topics.list_media(query)
    }
}
impl ChatroomsRepository for FaultingPostgresRepositories {
    fn list_chatrooms(
        &self,
        query: ListChatroomsQuery,
    ) -> ChatroomsRepositoryFuture<'_, ChatroomPage> {
        self.chatrooms.list_chatrooms(query)
    }
    fn message_history(
        &self,
        query: MessageHistoryQuery,
    ) -> ChatroomsRepositoryFuture<'_, MessageHistoryPage> {
        self.chatrooms.message_history(query)
    }
    fn mark_read<'a>(
        &'a self,
        transaction: &'a mut dyn jamye_server::ports::transactions::TransactionHandle,
        command: &'a MarkReadCommand,
    ) -> ChatroomsRepositoryFuture<'a, ReadMarker> {
        Box::pin(async move {
            let value = self.chatrooms.mark_read(transaction, command).await?;
            if self.faults.after(Boundary::MarkRead) {
                Err(ChatroomsRepositoryError::Unavailable)
            } else {
                Ok(value)
            }
        })
    }
    fn read_marker(
        &self,
        query: ReadMarkerQuery,
    ) -> ChatroomsRepositoryFuture<'_, Option<ReadMarker>> {
        self.chatrooms.read_marker(query)
    }
}
impl NotificationEventsRepository for FaultingPostgresRepositories {
    fn record_topic_created<'a>(
        &'a self,
        transaction: &'a mut dyn jamye_server::ports::transactions::TransactionHandle,
        command: &'a RecordTopicNotificationCommand,
    ) -> NotificationEventsRepositoryFuture<'a, NotificationFanoutReport> {
        Box::pin(async move {
            let value = self
                .notifications
                .record_topic_created(transaction, command)
                .await?;
            if self.faults.after(Boundary::TopicNotification) {
                Err(NotificationsRepositoryError::Unavailable)
            } else {
                Ok(value)
            }
        })
    }
    fn record_message_created<'a>(
        &'a self,
        transaction: &'a mut dyn jamye_server::ports::transactions::TransactionHandle,
        command: &'a RecordMessageNotificationCommand,
    ) -> NotificationEventsRepositoryFuture<'a, NotificationFanoutReport> {
        Box::pin(async move {
            let value = self
                .notifications
                .record_message_created(transaction, command)
                .await?;
            if self.faults.after(Boundary::MessageNotification) {
                Err(NotificationsRepositoryError::Unavailable)
            } else {
                Ok(value)
            }
        })
    }
    fn clear_topic_notifications<'a>(
        &'a self,
        transaction: &'a mut dyn jamye_server::ports::transactions::TransactionHandle,
        command: &'a ClearTopicNotificationsCommand,
    ) -> NotificationEventsRepositoryFuture<'a, NotificationClearReport> {
        Box::pin(async move {
            let value = self
                .notifications
                .clear_topic_notifications(transaction, command)
                .await?;
            if self.faults.after(Boundary::NotificationClear) {
                Err(NotificationsRepositoryError::Unavailable)
            } else {
                Ok(value)
            }
        })
    }
}
