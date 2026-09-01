fn mark_read_input() -> MarkConversationReadCompositionInput {
    MarkConversationReadCompositionInput {
        read: MarkReadCommand {
            marker_id: id(),
            user_id: id(),
            chatroom_id: id(),
            cursor: 1,
        },
    }
}
