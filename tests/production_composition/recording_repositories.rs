pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Step {
    MessageEventOutbox,
    MessageMediaBinding,
    MessageNotificationPush,
    TopicChatroomBootstrapAnnouncementRead,
    TopicNotificationPush,
    ReadMarker,
    NotificationClear,
}

#[derive(Default)]
struct RecorderState {
    trace: Vec<Step>,
    handles: Vec<usize>,
    fail_after: Option<Step>,
}

#[derive(Clone, Default)]
struct RecordingRepositories {
    state: Arc<Mutex<RecorderState>>,
}

impl RecordingRepositories {
    fn clear_failure(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.fail_after = None;
        state.trace.clear();
        state.handles.clear();
    }

    fn set_failure(&self, step: Step) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.fail_after = Some(step);
    }

    fn trace(&self) -> Vec<Step> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .trace
            .clone()
    }

    fn handles(&self) -> Vec<usize> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .handles
            .clone()
    }

    fn record(&self, handle: &mut dyn TransactionHandle, step: Step) -> bool {
        let handle_id = handle
            .as_any_mut()
            .downcast_mut::<RecordingHandle>()
            .map(|handle| handle.id)
            .unwrap_or_default();
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.trace.push(step);
        state.handles.push(handle_id);
        state.fail_after == Some(step)
    }
}

impl MessagingRepository for RecordingRepositories {
    fn send<'a>(
        &'a self,
        handle: &'a mut dyn TransactionHandle,
        command: &'a SendMessageCommand,
    ) -> MessagingFuture<'a, PersistMessageOutcome> {
        let failed = self.record(handle, Step::MessageEventOutbox);
        let message = canonical_message(command);
        // This is part of the simulated messaging operation result, not a
        // second caller-supplied notification input. It is intentionally not
        // the canonical message id so UoW tests detect the wrong identity.
        let source_event_id = Uuid::from_u128(2);
        Box::pin(async move {
            if failed {
                Err(MessagingRepositoryError::DatabaseUnavailable)
            } else {
                Ok(PersistMessageOutcome::Created(PersistedMessage::new(
                    message,
                    source_event_id,
                )))
            }
        })
    }

    fn events(&self, _query: DeltaQuery) -> MessagingFuture<'_, EventPage> {
        Box::pin(async { Err(MessagingRepositoryError::DatabaseUnavailable) })
    }
}

impl MediaRepository for RecordingRepositories {
    fn create_upload_intent<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _command: &'a CreateUploadIntentCommand,
    ) -> MediaRepositoryFuture<'a, UploadIntentRecord> {
        Box::pin(async { Err(MediaRepositoryError::Unavailable) })
    }

    fn prepare_upload_finalize<'a>(
        &'a self,
        _query: &'a PrepareUploadFinalizeQuery,
    ) -> MediaRepositoryFuture<'a, UploadFinalizePreparation> {
        Box::pin(async { Err(MediaRepositoryError::Unavailable) })
    }

    fn finalize_upload<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _command: &'a jamye_server::ports::media::FinalizeUploadCommand,
    ) -> MediaRepositoryFuture<'a, UploadFinalizeRecord> {
        Box::pin(async { Err(MediaRepositoryError::Unavailable) })
    }

    fn bind_message_media<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        _command: &'a BindMessageMediaCommand,
    ) -> MediaRepositoryFuture<'a, Vec<jamye_server::domain::messaging::MessageAttachment>> {
        let failed = self.record(transaction, Step::MessageMediaBinding);
        Box::pin(async move {
            if failed {
                Err(MediaRepositoryError::Unavailable)
            } else {
                Ok(Vec::new())
            }
        })
    }

    fn authorize_media_access<'a>(
        &'a self,
        _query: &'a AuthorizeMediaAccessQuery,
    ) -> MediaRepositoryFuture<'a, MediaAccessRecord> {
        Box::pin(async { Err(MediaRepositoryError::Unavailable) })
    }
}

impl TopicsRepository for RecordingRepositories {
    fn create_topic<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a CreateTopicCommand,
    ) -> TopicsRepositoryFuture<'a, CreateTopicOutcome> {
        let failed = self.record(transaction, Step::TopicChatroomBootstrapAnnouncementRead);
        let topic = topic_record(command);
        Box::pin(async move {
            if failed {
                Err(TopicsRepositoryError::Unavailable)
            } else {
                Ok(CreateTopicOutcome::Created(topic))
            }
        })
    }

    fn patch_topic<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _command: &'a PatchTopicCommand,
    ) -> TopicsRepositoryFuture<'a, TopicRecord> {
        Box::pin(async { Err(TopicsRepositoryError::Unavailable) })
    }

    fn promote_enriched<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _topic_id: Uuid,
    ) -> TopicsRepositoryFuture<'a, TopicStatus> {
        Box::pin(async { Err(TopicsRepositoryError::Unavailable) })
    }

    fn replace_tags<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        _command: &'a ReplaceTopicTagsCommand,
    ) -> TopicsRepositoryFuture<'a, TopicTagPage> {
        Box::pin(async { Err(TopicsRepositoryError::Unavailable) })
    }

    fn list_topics(&self, _query: ListTopicsQuery) -> TopicsRepositoryFuture<'_, TopicPage> {
        Box::pin(async { Err(TopicsRepositoryError::Unavailable) })
    }

    fn list_topic_dates(
        &self,
        _query: ListTopicDatesQuery,
    ) -> TopicsRepositoryFuture<'_, TopicDatePage> {
        Box::pin(async { Err(TopicsRepositoryError::Unavailable) })
    }

    fn get_topic(&self, _query: GetTopicQuery) -> TopicsRepositoryFuture<'_, TopicRecord> {
        Box::pin(async { Err(TopicsRepositoryError::Unavailable) })
    }

    fn list_tags(&self, _query: ListTopicTagsQuery) -> TopicsRepositoryFuture<'_, TopicTagPage> {
        Box::pin(async { Err(TopicsRepositoryError::Unavailable) })
    }

    fn list_media(
        &self,
        _query: ListTopicMediaQuery,
    ) -> TopicsRepositoryFuture<'_, TopicMediaPage> {
        Box::pin(async { Err(TopicsRepositoryError::Unavailable) })
    }
}

impl ChatroomsRepository for RecordingRepositories {
    fn list_chatrooms(
        &self,
        _query: ListChatroomsQuery,
    ) -> ChatroomsRepositoryFuture<'_, ChatroomPage> {
        Box::pin(async { Err(ChatroomsRepositoryError::Unavailable) })
    }

    fn message_history(
        &self,
        _query: MessageHistoryQuery,
    ) -> ChatroomsRepositoryFuture<'_, MessageHistoryPage> {
        Box::pin(async { Err(ChatroomsRepositoryError::Unavailable) })
    }

    fn mark_read<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a MarkReadCommand,
    ) -> ChatroomsRepositoryFuture<'a, ReadMarker> {
        let failed = self.record(transaction, Step::ReadMarker);
        let marker = ReadMarker {
            id: command.marker_id,
            user_id: command.user_id,
            chatroom_id: command.chatroom_id,
            last_read_cursor: command.cursor,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        };
        Box::pin(async move {
            if failed {
                Err(ChatroomsRepositoryError::Unavailable)
            } else {
                Ok(marker)
            }
        })
    }

    fn read_marker(
        &self,
        _query: ReadMarkerQuery,
    ) -> ChatroomsRepositoryFuture<'_, Option<ReadMarker>> {
        Box::pin(async { Err(ChatroomsRepositoryError::Unavailable) })
    }
}

impl NotificationEventsRepository for RecordingRepositories {
    fn record_topic_created<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        _command: &'a RecordTopicNotificationCommand,
    ) -> NotificationEventsRepositoryFuture<'a, NotificationFanoutReport> {
        let failed = self.record(transaction, Step::TopicNotificationPush);
        Box::pin(async move {
            if failed {
                Err(NotificationsRepositoryError::Unavailable)
            } else {
                Ok(NotificationFanoutReport {
                    notification_count: 1,
                    occurrence_count: 1,
                })
            }
        })
    }

    fn record_message_created<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        _command: &'a RecordMessageNotificationCommand,
    ) -> NotificationEventsRepositoryFuture<'a, NotificationFanoutReport> {
        let failed = self.record(transaction, Step::MessageNotificationPush);
        Box::pin(async move {
            if failed {
                Err(NotificationsRepositoryError::Unavailable)
            } else {
                Ok(NotificationFanoutReport {
                    notification_count: 1,
                    occurrence_count: 1,
                })
            }
        })
    }

    fn clear_topic_notifications<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        _command: &'a ClearTopicNotificationsCommand,
    ) -> NotificationEventsRepositoryFuture<'a, NotificationClearReport> {
        let failed = self.record(transaction, Step::NotificationClear);
        Box::pin(async move {
            if failed {
                Err(NotificationsRepositoryError::Unavailable)
            } else {
                Ok(NotificationClearReport { cleared_count: 1 })
            }
        })
    }
}

struct RecordingTransactions {
    begins: AtomicUsize,
    commits: AtomicUsize,
    rollbacks: AtomicUsize,
}

impl RecordingTransactions {
    fn counts(&self) -> (usize, usize, usize) {
        (
            self.begins.load(Ordering::SeqCst),
            self.commits.load(Ordering::SeqCst),
            self.rollbacks.load(Ordering::SeqCst),
        )
    }
}

impl TransactionManager for RecordingTransactions {
    fn begin(&self) -> TransactionFuture<'_, BoxTransactionHandle> {
        self.begins.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(Box::new(RecordingHandle { id: 1 }) as BoxTransactionHandle) })
    }

    fn commit<'a>(&'a self, handle: BoxTransactionHandle) -> TransactionFuture<'a, ()> {
        assert!(handle.into_any().downcast::<RecordingHandle>().is_ok());
        self.commits.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }

    fn rollback<'a>(&'a self, handle: BoxTransactionHandle) -> TransactionFuture<'a, ()> {
        assert!(handle.into_any().downcast::<RecordingHandle>().is_ok());
        self.rollbacks.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }
}

struct RecordingHandle {
    id: usize,
}

impl TransactionHandle for RecordingHandle {
    fn as_any_mut(&mut self) -> &mut (dyn Any + Send) {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }
}

struct RecordingFixture {
    compositions: TransactionCompositions,
    repositories: Arc<RecordingRepositories>,
    transactions: Arc<RecordingTransactions>,
}

impl RecordingFixture {
    fn new() -> Self {
        let repositories = Arc::new(RecordingRepositories::default());
        let transactions = Arc::new(RecordingTransactions {
            begins: AtomicUsize::new(0),
            commits: AtomicUsize::new(0),
            rollbacks: AtomicUsize::new(0),
        });
        let dependencies = TransactionCompositionDependencies {
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
            chatrooms: Arc::new(ChatroomsService::new(
                transactions.clone(),
                repositories.clone(),
            )),
            notifications: repositories.clone(),
        };
        Self {
            compositions: TransactionCompositions::new(dependencies),
            repositories,
            transactions,
        }
    }

    fn trace(&self) -> Vec<Step> {
        self.repositories.trace()
    }

    fn handle_ids(&self) -> Vec<usize> {
        self.repositories.handles()
    }

    fn transaction_counts(&self) -> (usize, usize, usize) {
        self.transactions.counts()
    }

    fn set_failure(&self, step: Step) {
        self.repositories.set_failure(step);
    }

    fn clear_failure(&self) {
        self.repositories.clear_failure();
    }
}

fn id() -> Uuid {
    Uuid::new_v4()
}

fn canonical_message(command: &SendMessageCommand) -> CanonicalMessage {
    CanonicalMessage {
        id: id(),
        chatroom_id: command.chatroom_id,
        sender_id: Some(command.sender_id),
        client_msg_id: Some(command.client_msg_id),
        body: command.body.clone(),
        message_type: MessageKind::User,
        created_at: OffsetDateTime::UNIX_EPOCH,
        media: Vec::new(),
    }
}
