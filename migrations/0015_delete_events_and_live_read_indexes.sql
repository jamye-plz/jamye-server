-- migration: 0015_delete_events_and_live_read_indexes
-- prerequisite: 0014_audit_columns_and_updated_at_triggers.sql
-- reversibility: forward-only; repair or evolve with a new numbered migration
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: allow irreversible topic content scrubbing and add live-row indexes used by message/topic delete reads
-- lock impact: brief metadata locks while replacing one check constraint and creating partial indexes; no data rewrite

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '120s';

ALTER TABLE topics
    DROP CONSTRAINT topics_title_check,
    ADD CONSTRAINT topics_title_check CHECK (
        (
            deleted_at IS NULL
            AND length(title) BETWEEN 1 AND 256
            AND title = btrim(title)
        )
        OR (
            deleted_at IS NOT NULL
            AND title = ''
        )
    );

CREATE INDEX ix_chatrooms_group_created_live
    ON chatrooms (group_id, created_at, id)
    WHERE deleted_at IS NULL;

CREATE INDEX ix_messages_chatroom_created_live
    ON messages (chatroom_id, created_at DESC, id DESC)
    WHERE deleted_at IS NULL;

CREATE INDEX ix_topics_group_created_live
    ON topics (group_id, created_at DESC, id DESC)
    WHERE deleted_at IS NULL;

CREATE INDEX ix_notifications_user_created_live
    ON notifications (user_id, created_at DESC, id DESC)
    WHERE deleted_at IS NULL;
