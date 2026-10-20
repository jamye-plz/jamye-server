-- migration: 0022_scrub_deleted_topic_tags
-- prerequisite: 0021_moderation_soft_delete.sql
-- reversibility: forward-only; the prior tag text is intentionally unrecoverable
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: scrub historical tag text and confidence for topics deleted before the runtime scrub covered soft-deleted tag rows
-- lock impact: row-level updates to affected topic_tags rows; no DDL

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '120s';

UPDATE topic_tags tag_entry
SET deleted_at = COALESCE(tag_entry.deleted_at, topic.deleted_at),
    tag = 'deleted-' || replace(tag_entry.id::text, '-', ''),
    confidence = NULL
FROM topics topic
WHERE topic.id = tag_entry.topic_id
  AND topic.deleted_at IS NOT NULL
  AND tag_entry.tag <> ('deleted-' || replace(tag_entry.id::text, '-', ''));
