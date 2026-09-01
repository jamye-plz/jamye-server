fn topic_record(command: &CreateTopicCommand) -> TopicRecord {
    TopicRecord {
        id: command.topic_id,
        group_id: command.group_id,
        author_id: command.author_id,
        author_nickname: "author".to_owned(),
        author_avatar_url: None,
        title: command.title.clone(),
        body: None,
        status: TopicStatus::Seed,
        tags: Vec::new(),
        media: Vec::new(),
        chatroom_id: command.topic_chatroom_id,
        unread: false,
        created_at: OffsetDateTime::UNIX_EPOCH,
        updated_at: OffsetDateTime::UNIX_EPOCH,
    }
}

fn send_message_input() -> SendMessageCompositionInput {
    SendMessageCompositionInput {
        message: SendMessageCommand {
            chatroom_id: id(),
            sender_id: id(),
            client_msg_id: id(),
            body: Some("Task-12 composition".to_owned()),
        },
        group_id: id(),
        topic_id: id(),
        sender_display_name: "sender".to_owned(),
        media: Vec::new(),
    }
}

fn create_topic_input() -> CreateTopicCompositionInput {
    CreateTopicCompositionInput {
        topic: CreateTopicCommand {
            topic_id: id(),
            topic_chatroom_id: id(),
            topic_event_id: id(),
            topic_outbox_id: id(),
            author_read_marker_id: id(),
            announcement_message_id: id(),
            announcement_client_msg_id: id(),
            announcement_event_id: id(),
            announcement_outbox_id: id(),
            group_id: id(),
            author_id: id(),
            idempotency_key: id(),
            request_fingerprint: "task-12".to_owned(),
            title: "Task-12 topic".to_owned(),
            announcement_body: "announcement".to_owned(),
        },
        author_display_name: "author".to_owned(),
    }
}
