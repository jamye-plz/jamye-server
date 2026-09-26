-- migration: 0012_remove_topic_media
-- prerequisite: 0011_main_chat_notifications.sql
-- reversibility: forward-only; repair or evolve with a new numbered migration
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: remove topic-scoped media and keep chat media as the only upload
--   consumer while preserving provider cleanup work for topic object keys
-- lock impact: deletes deprecated topic media rows and takes brief metadata
--   locks while dropping topic_media, bound_topic_media_id, and replacing the
--   media_uploads scope and consumer-shape checks

-- Intent ids are fresh UUIDs (as in account deletion) rather than reused row
-- ids, so no cross-table id collision can abort the migration.
WITH topic_objects AS (
    SELECT tm.object_key
    FROM topic_media AS tm
    WHERE tm.object_key = btrim(tm.object_key)
      AND length(tm.object_key) BETWEEN 1 AND 512
    UNION
    SELECT mu.object_key
    FROM media_uploads AS mu
    WHERE mu.scope = 'topic'
      AND mu.object_key = btrim(mu.object_key)
      AND length(mu.object_key) BETWEEN 1 AND 512
)
INSERT INTO account_object_deletion_intents (id, object_key)
SELECT gen_random_uuid(), object_key
FROM topic_objects
ON CONFLICT (object_key) DO NOTHING;

ALTER TABLE media_uploads
    DROP CONSTRAINT fk_media_uploads_bound_topic_media;

DELETE FROM topic_media;

DELETE FROM media_uploads
WHERE scope = 'topic';

-- The deletes above queue checks for the DEFERRABLE foreign keys; PostgreSQL
-- refuses DROP/ALTER on a table with pending trigger events (SQLSTATE 55006),
-- so run those checks now instead of at commit.
SET CONSTRAINTS ALL IMMEDIATE;

DROP TABLE topic_media;

ALTER TABLE media_uploads
    DROP CONSTRAINT uq_media_uploads_bound_topic_media,
    DROP CONSTRAINT uq_media_uploads_bound_topic_pair,
    DROP CONSTRAINT media_uploads_scope_check,
    DROP CONSTRAINT media_uploads_consumer_shape_check,
    DROP COLUMN bound_topic_media_id,
    ADD CONSTRAINT media_uploads_scope_check CHECK (scope = 'chat'),
    ADD CONSTRAINT media_uploads_consumer_shape_check CHECK (
        (
            status = 'pending'
            AND confirmed_at IS NULL
            AND consumed_at IS NULL
            AND bound_message_id IS NULL
        )
        OR (
            status = 'confirmed'
            AND scope = 'chat'
            AND confirmed_at IS NOT NULL
            AND consumed_at IS NULL
            AND bound_message_id IS NULL
        )
        OR (
            status = 'bound'
            AND scope = 'chat'
            AND confirmed_at IS NOT NULL
            AND consumed_at IS NOT NULL
            AND bound_message_id IS NOT NULL
        )
        OR (
            status = 'expired'
            AND consumed_at IS NULL
            AND bound_message_id IS NULL
        )
    );
