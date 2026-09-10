fn mark_read_input() -> MarkConversationReadCompositionInput {
    MarkConversationReadCompositionInput {
        read: MarkReadCommand {
            marker_id: id(),
            user_id: id(),
            chatroom_id: id(),
            anchor: ReadMarkerAnchor::Cursor(1),
        },
    }
}
