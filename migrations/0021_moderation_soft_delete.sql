-- migration: 0021_moderation_soft_delete
-- prerequisite: 0020_ugc_moderation.sql
-- reversibility: forward-only; repair or evolve with a new numbered migration
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: bring moderation blocks and reports under the D14 soft-delete lifecycle and the D19 updated_at convention while preserving existing keys and retained rows
-- lock impact: adding nullable columns is metadata-only; the two table-wide backfills update every existing moderation row, and SET NOT NULL validates each table under ACCESS EXCLUSIVE. The D19 triggers add no index build. Existing PK/status/target indexes continue to serve the same lookups with a live-row filter, so no replacement index is needed. lock_timeout/statement_timeout bound the attempt, and migrate runs before the api and worker start

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '120s';

ALTER TABLE user_blocks
    ADD COLUMN updated_at TIMESTAMPTZ,
    ADD COLUMN deleted_at TIMESTAMPTZ;
UPDATE user_blocks
SET updated_at = created_at
WHERE updated_at IS NULL;
ALTER TABLE user_blocks
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE reports
    ADD COLUMN updated_at TIMESTAMPTZ,
    ADD COLUMN deleted_at TIMESTAMPTZ;
UPDATE reports
SET updated_at = COALESCE(handled_at, created_at)
WHERE updated_at IS NULL;
ALTER TABLE reports
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL;

CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON user_blocks
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON reports
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
