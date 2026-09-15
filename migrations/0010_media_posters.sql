-- migration: 0010_media_posters
-- prerequisite: 0009_message_anchor_index.sql
-- reversibility: forward-only; repair or evolve with a new numbered migration
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: allow a confirmed chat video upload to carry an optional poster
--   image upload finalized independently; posters keep the existing
--   media_uploads status machine (confirmed -> bound at message binding),
--   they never own a message_media row of their own
-- lock impact: additive nullable column, an unnamed-default not-null-safe FK,
--   a self-reference check, and a partial unique index on media_uploads;
--   brief ACCESS EXCLUSIVE while the column/constraints are added, no rewrite
--   of existing rows since the column defaults to NULL

ALTER TABLE media_uploads
    ADD COLUMN poster_upload_id UUID NULL,
    ADD CONSTRAINT fk_media_uploads_poster_upload
        FOREIGN KEY (poster_upload_id) REFERENCES media_uploads (id),
    ADD CONSTRAINT media_uploads_poster_self_reference_check CHECK (
        poster_upload_id IS NULL OR poster_upload_id <> id
    );

CREATE UNIQUE INDEX uq_media_uploads_poster_upload
    ON media_uploads (poster_upload_id)
    WHERE poster_upload_id IS NOT NULL;
