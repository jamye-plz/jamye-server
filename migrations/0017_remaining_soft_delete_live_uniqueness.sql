-- migration: 0017_remaining_soft_delete_live_uniqueness
-- prerequisite: 0016_account_grace_period.sql
-- reversibility: forward-only; repair or evolve with a new numbered migration
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: finish task-14 phase-2 soft-delete semantics for remaining lifecycle rows, live-row uniqueness, and structural topic-announcement lookup
-- lock impact: replaces selected unique constraints/indexes with live-row partial unique indexes, adds one nullable message reference column, backfills announcement references from the legacy body format, and creates supporting live indexes under lock_timeout/statement_timeout

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '120s';

ALTER TABLE invites
    DROP CONSTRAINT uq_invites_code;
CREATE UNIQUE INDEX uq_invites_code
    ON invites (code)
    WHERE deleted_at IS NULL;

ALTER TABLE chatroom_reads
    DROP CONSTRAINT uq_chatroom_reads_user_chatroom;
CREATE UNIQUE INDEX uq_chatroom_reads_user_chatroom
    ON chatroom_reads (user_id, chatroom_id)
    WHERE deleted_at IS NULL;

ALTER TABLE topic_tags
    DROP CONSTRAINT uq_topic_tags_topic_tag;
CREATE UNIQUE INDEX uq_topic_tags_topic_tag
    ON topic_tags (topic_id, tag)
    WHERE deleted_at IS NULL;

DROP INDEX ux_notifications_user_dedup;
CREATE UNIQUE INDEX ux_notifications_user_dedup
    ON notifications (user_id, dedup_key)
    WHERE dedup_key IS NOT NULL AND deleted_at IS NULL;

ALTER TABLE push_installations
    DROP CONSTRAINT uq_push_installations_installation_id,
    DROP CONSTRAINT uq_push_installations_destination;
CREATE UNIQUE INDEX uq_push_installations_installation_id
    ON push_installations (installation_id)
    WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX uq_push_installations_destination
    ON push_installations (environment, token)
    WHERE deleted_at IS NULL;

ALTER TABLE push_delivery_intents
    DROP CONSTRAINT uq_push_delivery_source_installation;
CREATE UNIQUE INDEX uq_push_delivery_source_installation
    ON push_delivery_intents (source_event_id, push_installation_id)
    WHERE deleted_at IS NULL;

ALTER TABLE messages
    ADD COLUMN announcement_for_topic_id UUID,
    ADD CONSTRAINT fk_messages_announcement_for_topic
        FOREIGN KEY (announcement_for_topic_id) REFERENCES topics (id);

WITH candidates AS (
    SELECT message.id AS message_id,
           topic.id AS topic_id,
           count(*) OVER (PARTITION BY topic.id) AS topic_match_count,
           count(*) OVER (PARTITION BY message.id) AS message_match_count
    FROM topics topic
    JOIN chatrooms main_chatroom
      ON main_chatroom.group_id = topic.group_id
     AND main_chatroom.type = 'main'
     AND main_chatroom.topic_id IS NULL
     AND main_chatroom.deleted_at IS NULL
    JOIN messages message
      ON message.chatroom_id = main_chatroom.id
     AND message.sender_id = topic.author_id
     AND message.type = 'user'
     AND message.deleted_at IS NULL
     AND message.body LIKE '새로운 주제를 올렸어요: %](/groups/'
        || topic.group_id::text || '/topics/' || topic.id::text || '/chat)'
    WHERE topic.deleted_at IS NULL
)
UPDATE messages message
SET announcement_for_topic_id = candidates.topic_id
FROM candidates
WHERE message.id = candidates.message_id
  AND candidates.topic_match_count = 1
  AND candidates.message_match_count = 1;

CREATE UNIQUE INDEX uq_messages_live_announcement_topic
    ON messages (announcement_for_topic_id)
    WHERE announcement_for_topic_id IS NOT NULL AND deleted_at IS NULL;

CREATE INDEX ix_invites_group_created_live
    ON invites (group_id, created_at, id)
    WHERE deleted_at IS NULL;

CREATE INDEX ix_chatroom_reads_live_user_chatroom
    ON chatroom_reads (user_id, chatroom_id, last_read_cursor)
    WHERE deleted_at IS NULL;

CREATE INDEX ix_topic_tags_live_topic_tag
    ON topic_tags (topic_id, tag, id)
    WHERE deleted_at IS NULL;

CREATE INDEX ix_push_installations_live_user
    ON push_installations (user_id, disabled_at, last_seen_at DESC, id)
    WHERE deleted_at IS NULL;

CREATE INDEX ix_push_delivery_live_due
    ON push_delivery_intents (
        (COALESCE(next_attempt_at, created_at)),
        created_at,
        id
    )
    WHERE deleted_at IS NULL AND status IN ('pending', 'retryable');
