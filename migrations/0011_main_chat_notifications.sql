-- chat_unread notifications may now belong to a group's main chatroom, which
-- has no topic. Topic-scoped notification shapes are unchanged; only the
-- topic_id requirement is relaxed for chat_unread rows.
ALTER TABLE notifications DROP CONSTRAINT notifications_topic_shape_check;
ALTER TABLE notifications ADD CONSTRAINT notifications_topic_shape_check CHECK (
    type = 'other'
    OR (
        conversation_id IS NOT NULL
        AND source_cursor IS NOT NULL
        AND dedup_key IS NOT NULL
        AND (type = 'chat_unread' OR topic_id IS NOT NULL)
    )
);

-- Read-marker clearing now runs per conversation (main or topic).
CREATE INDEX ix_notifications_conversation_cursor
    ON notifications (user_id, conversation_id, source_cursor, id)
    WHERE conversation_id IS NOT NULL AND source_cursor IS NOT NULL;
