-- migration: 0014_audit_columns_and_updated_at_triggers
-- prerequisite: 0013_https_avatar_urls.sql
-- reversibility: forward-only; repair or evolve with a new numbered migration
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: add uniform audit columns and D19 updated_at behavior before soft-delete reads change
-- lock impact: each table's ALTER COLUMN ... SET NOT NULL takes ACCESS EXCLUSIVE and validates existing rows with a table scan after the backfill; lock_timeout 5s and statement_timeout 120s bound the attempt, and current production tables are small (messages 72, conversation_events 76, outbox_events 76, refresh_sessions 1040); no indexes, filters, uniqueness rewrites, or row-hiding predicates are introduced

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '120s';

ALTER TABLE users
    ADD COLUMN updated_at TIMESTAMPTZ,
    ADD COLUMN deleted_at TIMESTAMPTZ;
UPDATE users SET updated_at = created_at WHERE updated_at IS NULL;
ALTER TABLE users
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE groups
    ADD COLUMN updated_at TIMESTAMPTZ;
UPDATE groups SET updated_at = created_at WHERE updated_at IS NULL;
ALTER TABLE groups
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE memberships
    ADD COLUMN created_at TIMESTAMPTZ,
    ADD COLUMN updated_at TIMESTAMPTZ,
    ADD COLUMN deleted_at TIMESTAMPTZ;
UPDATE memberships
SET created_at = joined_at,
    updated_at = joined_at
WHERE created_at IS NULL OR updated_at IS NULL;
ALTER TABLE memberships
    ALTER COLUMN created_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN created_at SET NOT NULL,
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE chatrooms
    ADD COLUMN updated_at TIMESTAMPTZ,
    ADD COLUMN deleted_at TIMESTAMPTZ;
UPDATE chatrooms SET updated_at = created_at WHERE updated_at IS NULL;
ALTER TABLE chatrooms
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE messages
    ADD COLUMN updated_at TIMESTAMPTZ,
    ADD COLUMN deleted_at TIMESTAMPTZ;
UPDATE messages SET updated_at = created_at WHERE updated_at IS NULL;
ALTER TABLE messages
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE conversation_events
    ADD COLUMN created_at TIMESTAMPTZ,
    ADD COLUMN updated_at TIMESTAMPTZ,
    ADD COLUMN deleted_at TIMESTAMPTZ;
UPDATE conversation_events
SET created_at = occurred_at,
    updated_at = occurred_at
WHERE created_at IS NULL OR updated_at IS NULL;
ALTER TABLE conversation_events
    ALTER COLUMN created_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN created_at SET NOT NULL,
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE outbox_events
    ADD COLUMN updated_at TIMESTAMPTZ,
    ADD COLUMN deleted_at TIMESTAMPTZ;
UPDATE outbox_events
SET updated_at = COALESCE(published_at, dead_lettered_at, next_attempt_at, claim_expires_at, created_at)
WHERE updated_at IS NULL;
ALTER TABLE outbox_events
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE auth_identities
    ADD COLUMN updated_at TIMESTAMPTZ,
    ADD COLUMN deleted_at TIMESTAMPTZ;
UPDATE auth_identities SET updated_at = created_at WHERE updated_at IS NULL;
ALTER TABLE auth_identities
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE refresh_sessions
    ADD COLUMN updated_at TIMESTAMPTZ,
    ADD COLUMN deleted_at TIMESTAMPTZ;
UPDATE refresh_sessions
SET updated_at = COALESCE(revoked_at, consumed_at, created_at)
WHERE updated_at IS NULL;
ALTER TABLE refresh_sessions
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE invites
    ADD COLUMN updated_at TIMESTAMPTZ,
    ADD COLUMN deleted_at TIMESTAMPTZ;
UPDATE invites SET updated_at = created_at WHERE updated_at IS NULL;
ALTER TABLE invites
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE chatroom_reads
    ADD COLUMN created_at TIMESTAMPTZ,
    ADD COLUMN deleted_at TIMESTAMPTZ;
UPDATE chatroom_reads SET created_at = updated_at WHERE created_at IS NULL;
ALTER TABLE chatroom_reads
    ALTER COLUMN created_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN created_at SET NOT NULL;

ALTER TABLE topics
    ADD COLUMN deleted_at TIMESTAMPTZ;

ALTER TABLE topic_tags
    ADD COLUMN created_at TIMESTAMPTZ,
    ADD COLUMN updated_at TIMESTAMPTZ,
    ADD COLUMN deleted_at TIMESTAMPTZ;
UPDATE topic_tags AS tt
SET created_at = t.created_at,
    updated_at = t.updated_at
FROM topics AS t
WHERE tt.topic_id = t.id
  AND (tt.created_at IS NULL OR tt.updated_at IS NULL);
UPDATE topic_tags
SET created_at = COALESCE(created_at, clock_timestamp()),
    updated_at = COALESCE(updated_at, created_at, clock_timestamp())
WHERE created_at IS NULL OR updated_at IS NULL;
ALTER TABLE topic_tags
    ALTER COLUMN created_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN created_at SET NOT NULL,
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE media_uploads
    ADD COLUMN updated_at TIMESTAMPTZ,
    ADD COLUMN deleted_at TIMESTAMPTZ;
UPDATE media_uploads
SET updated_at = COALESCE(consumed_at, confirmed_at, created_at)
WHERE updated_at IS NULL;
ALTER TABLE media_uploads
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE message_media
    ADD COLUMN updated_at TIMESTAMPTZ,
    ADD COLUMN deleted_at TIMESTAMPTZ;
UPDATE message_media SET updated_at = created_at WHERE updated_at IS NULL;
ALTER TABLE message_media
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE notifications
    ADD COLUMN updated_at TIMESTAMPTZ,
    ADD COLUMN deleted_at TIMESTAMPTZ;
UPDATE notifications
SET updated_at = COALESCE(read_at, created_at)
WHERE updated_at IS NULL;
ALTER TABLE notifications
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE push_installations
    ADD COLUMN created_at TIMESTAMPTZ,
    ADD COLUMN updated_at TIMESTAMPTZ,
    ADD COLUMN deleted_at TIMESTAMPTZ;
UPDATE push_installations
SET created_at = last_seen_at,
    updated_at = COALESCE(disabled_at, last_seen_at)
WHERE created_at IS NULL OR updated_at IS NULL;
ALTER TABLE push_installations
    ALTER COLUMN created_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN created_at SET NOT NULL,
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE push_delivery_intents
    ADD COLUMN updated_at TIMESTAMPTZ,
    ADD COLUMN deleted_at TIMESTAMPTZ;
UPDATE push_delivery_intents
SET updated_at = COALESCE(
        succeeded_at,
        failed_at,
        dead_lettered_at,
        next_attempt_at,
        lease_expires_at,
        created_at
    )
WHERE updated_at IS NULL;
ALTER TABLE push_delivery_intents
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE anonymous_author_tombstones
    ADD COLUMN updated_at TIMESTAMPTZ,
    ADD COLUMN deleted_at TIMESTAMPTZ;
UPDATE anonymous_author_tombstones SET updated_at = created_at WHERE updated_at IS NULL;
ALTER TABLE anonymous_author_tombstones
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE account_object_deletion_intents
    ADD COLUMN updated_at TIMESTAMPTZ,
    ADD COLUMN deleted_at TIMESTAMPTZ;
UPDATE account_object_deletion_intents
SET updated_at = COALESCE(
        succeeded_at,
        failed_at,
        dead_lettered_at,
        next_attempt_at,
        lease_expires_at,
        created_at
    )
WHERE updated_at IS NULL;
ALTER TABLE account_object_deletion_intents
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL;

CREATE OR REPLACE FUNCTION jamye_set_updated_at_if_changed()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW IS DISTINCT FROM OLD
       AND NEW.updated_at IS NOT DISTINCT FROM OLD.updated_at THEN
        NEW.updated_at = clock_timestamp();
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON groups
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON memberships
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON chatrooms
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON messages
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON conversation_events
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON outbox_events
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON auth_identities
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON refresh_sessions
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON invites
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON chatroom_reads
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON topics
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON topic_tags
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON media_uploads
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON message_media
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON notifications
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON push_installations
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON push_delivery_intents
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON anonymous_author_tombstones
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON account_object_deletion_intents
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
