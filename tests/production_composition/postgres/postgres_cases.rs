#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Boundary {
    SendMessage,
    MessageMedia,
    MessageNotification,
    CreateTopic,
    TopicNotification,
    MarkRead,
    NotificationClear,
}

#[tokio::test]
async fn postgres_send_message_fault_after_messaging_rolls_back_then_retries() -> TestResult {
    assert_boundary(Boundary::SendMessage).await
}
#[tokio::test]
async fn postgres_send_message_fault_after_media_rolls_back_then_retries() -> TestResult {
    assert_boundary(Boundary::MessageMedia).await
}
#[tokio::test]
async fn postgres_send_message_fault_after_notification_rolls_back_then_retries() -> TestResult {
    assert_boundary(Boundary::MessageNotification).await
}
#[tokio::test]
async fn postgres_create_topic_fault_after_core_rolls_back_then_retries() -> TestResult {
    assert_boundary(Boundary::CreateTopic).await
}
#[tokio::test]
async fn postgres_create_topic_fault_after_notification_rolls_back_then_retries() -> TestResult {
    assert_boundary(Boundary::TopicNotification).await
}
#[tokio::test]
async fn postgres_mark_read_fault_after_marker_rolls_back_then_retries() -> TestResult {
    assert_boundary(Boundary::MarkRead).await
}
#[tokio::test]
async fn postgres_mark_read_fault_after_clear_rolls_back_then_retries() -> TestResult {
    assert_boundary(Boundary::NotificationClear).await
}

#[tokio::test]
async fn postgres_message_persistence_returns_the_canonical_event_id_for_first_and_retry()
-> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let fixture = PostgresFixture::new(pool.clone()).await?;
        let client_msg_id = Uuid::new_v4();
        let first = fixture.persist_message(client_msg_id).await?;
        let retry = fixture.persist_message(client_msg_id).await?;
        let canonical_event_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM conversation_events WHERE conversation_id = $1 AND event_type = 'message.created' AND event_version = 1 AND payload ->> 'id' = $2::uuid::text",
        )
        .bind(fixture.topic_chatroom_id)
        .bind(first.message().id)
        .fetch_one(&pool)
        .await?;
        require(
            first.source_event_id() == canonical_event_id && retry.source_event_id() == canonical_event_id,
            &format!("Task-12 RED: persisted source event differs: first={}, retry={}, canonical={canonical_event_id}", first.source_event_id(), retry.source_event_id()),
        )
    }.await;
    pool.close().await;
    database.dispose().await?;
    result
}

async fn assert_boundary(boundary: Boundary) -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let fixture = PostgresFixture::new(pool.clone()).await?;
        let baseline = DurableSnapshot::load(&pool, fixture.audio_upload_id).await?;
        fixture.arm(boundary);
        require(
            fixture.run(boundary).await.is_err(),
            "armed composition unexpectedly succeeded",
        )?;
        require(
            fixture.reached(boundary),
            "delegating fault was not reached",
        )?;
        require_eq(
            DurableSnapshot::load(&pool, fixture.audio_upload_id).await?,
            baseline,
            "forced post-operation failure left durable partial state",
        )?;
        fixture.disarm();
        require(
            fixture.run(boundary).await.is_ok(),
            "clean retry did not commit",
        )?;
        require_eq(
            DurableSnapshot::load(&pool, fixture.audio_upload_id)
                .await?
                .delta(baseline),
            boundary.expected_delta(),
            "clean retry durable row delta differed or duplicated state",
        )
    }
    .await;
    pool.close().await;
    database.dispose().await?;
    result
}

impl Boundary {
    fn expected_delta(self) -> DurableSnapshot {
        match self {
            Self::SendMessage | Self::MessageMedia | Self::MessageNotification => DurableSnapshot {
                messages: 1,
                conversation_events: 1,
                outbox_events: 1,
                message_media: 1,
                topics: 0,
                chatrooms: 0,
                chatroom_reads: 0,
                notifications: 1,
                unread_notifications: 1,
                push_delivery_intents: 1,
                audio_upload_confirmed_unbound: -1,
                audio_upload_bound_consumed: 1,
            },
            Self::CreateTopic | Self::TopicNotification => DurableSnapshot {
                messages: 1,
                conversation_events: 2,
                outbox_events: 2,
                message_media: 0,
                topics: 1,
                chatrooms: 1,
                chatroom_reads: 1,
                notifications: 1,
                unread_notifications: 1,
                push_delivery_intents: 1,
                audio_upload_confirmed_unbound: 0,
                audio_upload_bound_consumed: 0,
            },
            Self::MarkRead | Self::NotificationClear => DurableSnapshot {
                messages: 0,
                conversation_events: 0,
                outbox_events: 0,
                message_media: 0,
                topics: 0,
                chatrooms: 0,
                chatroom_reads: 1,
                notifications: 0,
                unread_notifications: -1,
                push_delivery_intents: 0,
                audio_upload_confirmed_unbound: 0,
                audio_upload_bound_consumed: 0,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DurableSnapshot {
    messages: i64,
    conversation_events: i64,
    outbox_events: i64,
    message_media: i64,
    topics: i64,
    chatrooms: i64,
    chatroom_reads: i64,
    notifications: i64,
    unread_notifications: i64,
    push_delivery_intents: i64,
    audio_upload_confirmed_unbound: i64,
    audio_upload_bound_consumed: i64,
}

impl DurableSnapshot {
    async fn load(pool: &PgPool, audio_upload_id: Uuid) -> TestResult<Self> {
        Ok(Self {
            messages: count(pool, "messages").await?,
            conversation_events: count(pool, "conversation_events").await?,
            outbox_events: count(pool, "outbox_events").await?,
            message_media: count(pool, "message_media").await?,
            topics: count(pool, "topics").await?,
            chatrooms: count(pool, "chatrooms").await?,
            chatroom_reads: count(pool, "chatroom_reads").await?,
            notifications: count(pool, "notifications").await?,
            unread_notifications: count(pool, "notifications WHERE read_at IS NULL").await?,
            push_delivery_intents: count(pool, "push_delivery_intents").await?,
            audio_upload_confirmed_unbound: audio_upload_state(
                pool,
                audio_upload_id,
                "confirmed",
                false,
            )
            .await?,
            audio_upload_bound_consumed: audio_upload_state(pool, audio_upload_id, "bound", true)
                .await?,
        })
    }
    fn delta(self, baseline: Self) -> Self {
        Self {
            messages: self.messages - baseline.messages,
            conversation_events: self.conversation_events - baseline.conversation_events,
            outbox_events: self.outbox_events - baseline.outbox_events,
            message_media: self.message_media - baseline.message_media,
            topics: self.topics - baseline.topics,
            chatrooms: self.chatrooms - baseline.chatrooms,
            chatroom_reads: self.chatroom_reads - baseline.chatroom_reads,
            notifications: self.notifications - baseline.notifications,
            unread_notifications: self.unread_notifications - baseline.unread_notifications,
            push_delivery_intents: self.push_delivery_intents - baseline.push_delivery_intents,
            audio_upload_confirmed_unbound: self.audio_upload_confirmed_unbound
                - baseline.audio_upload_confirmed_unbound,
            audio_upload_bound_consumed: self.audio_upload_bound_consumed
                - baseline.audio_upload_bound_consumed,
        }
    }
}

#[derive(Clone, Default)]
struct Faults(Arc<Mutex<FaultState>>);
#[derive(Default)]
struct FaultState {
    armed: Option<Boundary>,
    reached: bool,
}
impl Faults {
    fn arm(&self, boundary: Boundary) {
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = FaultState {
            armed: Some(boundary),
            reached: false,
        };
    }
    fn clear(&self) {
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = FaultState::default();
    }
    fn after(&self, boundary: Boundary) -> bool {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if state.armed == Some(boundary) {
            state.reached = true;
            true
        } else {
            false
        }
    }
    fn reached(&self, boundary: Boundary) -> bool {
        let state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        state.armed == Some(boundary) && state.reached
    }
}

pub(crate) struct PostgresFixture {
    pub(crate) pool: PgPool,
    compositions: TransactionCompositions,
    faults: Faults,
    pub(crate) recipient_id: Uuid,
    pub(crate) main_chatroom_id: Uuid,
    pub(crate) topic_chatroom_id: Uuid,
    pub(crate) audio_upload_id: Uuid,
    pub(crate) push_installation_id: Uuid,
    pub(crate) seeded_notification_id: Uuid,
    pub(crate) send: SendMessageCompositionInput,
    pub(crate) topic: CreateTopicCompositionInput,
    pub(crate) read: MarkConversationReadCompositionInput,
}
