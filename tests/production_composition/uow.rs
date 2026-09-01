use super::*;

#[tokio::test]
async fn send_message_calls_every_feature_operation_on_one_handle_then_commits_once() {
    let fixture = RecordingFixture::new();
    let result = fixture
        .compositions
        .send_message(send_message_input())
        .await;

    assert!(result.is_ok(), "SendMessage composition must complete");
    assert_eq!(
        fixture.trace(),
        vec![
            Step::MessageEventOutbox,
            Step::MessageMediaBinding,
            Step::MessageNotificationPush,
        ],
        "SendMessage must preserve message -> media -> notification order",
    );
    assert_eq!(fixture.handle_ids(), vec![1, 1, 1]);
    assert_eq!(fixture.transaction_counts(), (1, 1, 0));
}

#[tokio::test]
async fn create_topic_calls_every_feature_operation_on_one_handle_then_commits_once() {
    let fixture = RecordingFixture::new();
    let result = fixture
        .compositions
        .create_topic(create_topic_input())
        .await;

    assert!(result.is_ok(), "CreateTopic composition must complete");
    assert_eq!(
        fixture.trace(),
        vec![
            Step::TopicChatroomBootstrapAnnouncementRead,
            Step::TopicNotificationPush,
        ],
        "CreateTopic must retain task-7 bootstrap before task-9 notification",
    );
    assert_eq!(fixture.handle_ids(), vec![1, 1]);
    assert_eq!(fixture.transaction_counts(), (1, 1, 0));
}

#[tokio::test]
async fn mark_conversation_read_calls_every_feature_operation_on_one_handle_then_commits_once() {
    let fixture = RecordingFixture::new();
    let result = fixture
        .compositions
        .mark_conversation_read(mark_read_input())
        .await;

    assert!(
        result.is_ok(),
        "MarkConversationRead composition must complete"
    );
    assert_eq!(
        fixture.trace(),
        vec![Step::ReadMarker, Step::NotificationClear],
        "MarkConversationRead must advance the marker before bounded notification clear",
    );
    assert_eq!(fixture.handle_ids(), vec![1, 1]);
    assert_eq!(fixture.transaction_counts(), (1, 1, 0));
}

#[tokio::test]
async fn send_message_failure_after_message_rolls_back_then_clean_retry_commits() {
    let fixture = RecordingFixture::new();
    fixture.set_failure(Step::MessageEventOutbox);
    assert_send_failure_then_clean_retry(&fixture, Step::MessageEventOutbox).await;
}

#[tokio::test]
async fn send_message_failure_after_media_rolls_back_then_clean_retry_commits() {
    let fixture = RecordingFixture::new();
    fixture.set_failure(Step::MessageMediaBinding);
    assert_send_failure_then_clean_retry(&fixture, Step::MessageMediaBinding).await;
}

#[tokio::test]
async fn send_message_failure_after_notification_rolls_back_then_clean_retry_commits() {
    let fixture = RecordingFixture::new();
    fixture.set_failure(Step::MessageNotificationPush);
    assert_send_failure_then_clean_retry(&fixture, Step::MessageNotificationPush).await;
}

#[tokio::test]
async fn create_topic_failure_after_core_rolls_back_then_clean_retry_commits() {
    let fixture = RecordingFixture::new();
    fixture.set_failure(Step::TopicChatroomBootstrapAnnouncementRead);
    assert_create_topic_failure_then_clean_retry(
        &fixture,
        Step::TopicChatroomBootstrapAnnouncementRead,
    )
    .await;
}

#[tokio::test]
async fn create_topic_failure_after_notification_rolls_back_then_clean_retry_commits() {
    let fixture = RecordingFixture::new();
    fixture.set_failure(Step::TopicNotificationPush);
    assert_create_topic_failure_then_clean_retry(&fixture, Step::TopicNotificationPush).await;
}

#[tokio::test]
async fn mark_read_failure_after_marker_rolls_back_then_clean_retry_commits() {
    let fixture = RecordingFixture::new();
    fixture.set_failure(Step::ReadMarker);
    assert_mark_read_failure_then_clean_retry(&fixture, Step::ReadMarker).await;
}

#[tokio::test]
async fn mark_read_failure_after_notification_clear_rolls_back_then_clean_retry_commits() {
    let fixture = RecordingFixture::new();
    fixture.set_failure(Step::NotificationClear);
    assert_mark_read_failure_then_clean_retry(&fixture, Step::NotificationClear).await;
}

#[test]
fn send_message_derives_notification_identity_from_the_message_operation() {
    let input = send_message_input();
    let canonical_message = CanonicalMessage {
        id: id(),
        chatroom_id: input.message.chatroom_id,
        sender_id: Some(input.message.sender_id),
        client_msg_id: Some(input.message.client_msg_id),
        body: input.message.body.clone(),
        message_type: MessageKind::User,
        created_at: OffsetDateTime::UNIX_EPOCH,
        media: Vec::new(),
    };
    let persisted = PersistedMessage::new(canonical_message.clone(), Uuid::from_u128(3));
    let notification = input.notification_for(&persisted);
    assert!(notification.is_ok());
    let notification = match notification {
        Ok(notification) => notification,
        Err(_) => unreachable!("coherent message output must build one notification command"),
    };

    assert_eq!(notification.conversation_id, canonical_message.chatroom_id);
    assert_eq!(notification.sender_id, input.message.sender_id);
    assert_eq!(notification.source_message_id, canonical_message.id);
    assert_eq!(notification.source_event_id, persisted.source_event_id());
    assert_ne!(notification.source_event_id, notification.source_message_id);

    let incoherent_message = CanonicalMessage {
        chatroom_id: id(),
        ..canonical_message
    };
    assert_eq!(
        input.notification_for(&PersistedMessage::new(
            incoherent_message,
            persisted.source_event_id(),
        )),
        Err(TransactionCompositionError::IdentifierMismatch),
    );
}

#[test]
fn create_topic_derives_notification_identity_from_the_topic_operation() {
    let input = create_topic_input();
    let notification = input.notification_for();

    assert_eq!(notification.group_id, input.topic.group_id);
    assert_eq!(notification.topic_id, input.topic.topic_id);
    assert_eq!(notification.conversation_id, input.topic.topic_chatroom_id);
    assert_eq!(notification.source_event_id, input.topic.topic_event_id);
    assert_eq!(notification.author_id, input.topic.author_id);
}

#[test]
fn mark_read_derives_notification_clear_identity_from_the_marker_operation() {
    let input = mark_read_input();
    let clear = input.notification_clear();

    assert_eq!(clear.user_id, input.read.user_id);
    assert_eq!(clear.conversation_id, input.read.chatroom_id);
}

async fn assert_send_failure_then_clean_retry(fixture: &RecordingFixture, step: Step) {
    let result = fixture
        .compositions
        .send_message(send_message_input())
        .await;
    assert!(result.is_err());
    assert_eq!(fixture.trace(), send_prefix(step));
    assert_eq!(fixture.handle_ids(), vec![1; send_prefix(step).len()]);
    assert_eq!(fixture.transaction_counts(), (1, 0, 1));

    fixture.clear_failure();
    let retry = fixture
        .compositions
        .send_message(send_message_input())
        .await;
    assert!(retry.is_ok(), "clean SendMessage retry must commit");
    assert_eq!(
        fixture.trace(),
        vec![
            Step::MessageEventOutbox,
            Step::MessageMediaBinding,
            Step::MessageNotificationPush,
        ],
    );
    assert_eq!(fixture.handle_ids(), vec![1, 1, 1]);
    assert_eq!(fixture.transaction_counts(), (2, 1, 1));
}

async fn assert_create_topic_failure_then_clean_retry(fixture: &RecordingFixture, step: Step) {
    let result = fixture
        .compositions
        .create_topic(create_topic_input())
        .await;
    assert!(result.is_err());
    assert_eq!(fixture.trace(), create_topic_prefix(step));
    assert_eq!(
        fixture.handle_ids(),
        vec![1; create_topic_prefix(step).len()]
    );
    assert_eq!(fixture.transaction_counts(), (1, 0, 1));

    fixture.clear_failure();
    let retry = fixture
        .compositions
        .create_topic(create_topic_input())
        .await;
    assert!(retry.is_ok(), "clean CreateTopic retry must commit");
    assert_eq!(
        fixture.trace(),
        vec![
            Step::TopicChatroomBootstrapAnnouncementRead,
            Step::TopicNotificationPush,
        ],
    );
    assert_eq!(fixture.handle_ids(), vec![1, 1]);
    assert_eq!(fixture.transaction_counts(), (2, 1, 1));
}

async fn assert_mark_read_failure_then_clean_retry(fixture: &RecordingFixture, step: Step) {
    let result = fixture
        .compositions
        .mark_conversation_read(mark_read_input())
        .await;
    assert!(result.is_err());
    assert_eq!(fixture.trace(), mark_read_prefix(step));
    assert_eq!(fixture.handle_ids(), vec![1; mark_read_prefix(step).len()]);
    assert_eq!(fixture.transaction_counts(), (1, 0, 1));

    fixture.clear_failure();
    let retry = fixture
        .compositions
        .mark_conversation_read(mark_read_input())
        .await;
    assert!(
        retry.is_ok(),
        "clean MarkConversationRead retry must commit"
    );
    assert_eq!(
        fixture.trace(),
        vec![Step::ReadMarker, Step::NotificationClear],
    );
    assert_eq!(fixture.handle_ids(), vec![1, 1]);
    assert_eq!(fixture.transaction_counts(), (2, 1, 1));
}

fn send_prefix(step: Step) -> Vec<Step> {
    match step {
        Step::MessageEventOutbox => vec![Step::MessageEventOutbox],
        Step::MessageMediaBinding => vec![Step::MessageEventOutbox, Step::MessageMediaBinding],
        Step::MessageNotificationPush => vec![
            Step::MessageEventOutbox,
            Step::MessageMediaBinding,
            Step::MessageNotificationPush,
        ],
        _ => Vec::new(),
    }
}

fn create_topic_prefix(step: Step) -> Vec<Step> {
    match step {
        Step::TopicChatroomBootstrapAnnouncementRead => {
            vec![Step::TopicChatroomBootstrapAnnouncementRead]
        }
        Step::TopicNotificationPush => vec![
            Step::TopicChatroomBootstrapAnnouncementRead,
            Step::TopicNotificationPush,
        ],
        _ => Vec::new(),
    }
}

fn mark_read_prefix(step: Step) -> Vec<Step> {
    match step {
        Step::ReadMarker => vec![Step::ReadMarker],
        Step::NotificationClear => vec![Step::ReadMarker, Step::NotificationClear],
        _ => Vec::new(),
    }
}
