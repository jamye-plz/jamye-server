-- migration: 0019_user_avatar_uploads
-- prerequisite: 0018_add_apple_auth_identity_provider.sql
-- reversibility: forward-only; repair or evolve with a new numbered migration
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: track server-hosted profile avatar uploads (pending, active, released) so each user has at most one pending and one active avatar object and replaced objects can be queued for deletion
-- lock impact: additive relation, indexes, and trigger only; no existing table is altered, rewritten, or backfilled. The foreign key to users briefly takes a SHARE ROW EXCLUSIVE lock on users while the new empty table is created, which is harmless because migrate runs before the api and worker start

-- The content type is always image/jpeg and there is no separate confirmation
-- timestamp: a row is `pending` until finalize activates it, then `released`
-- once it is replaced, cleared, or superseded. The object key never leaves the
-- server; the public URL is derived from the row id only.
CREATE TABLE user_avatar_uploads (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users (id),
    object_key VARCHAR(512) NOT NULL,
    byte_size INTEGER NOT NULL,
    status VARCHAR(16) NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    released_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    deleted_at TIMESTAMPTZ,
    CONSTRAINT uq_user_avatar_uploads_object_key UNIQUE (object_key),
    CONSTRAINT user_avatar_uploads_byte_size_check CHECK (
        byte_size BETWEEN 1 AND 1048576
    ),
    CONSTRAINT user_avatar_uploads_status_check CHECK (
        status IN ('pending', 'active', 'released')
    )
);

CREATE UNIQUE INDEX uq_user_avatar_uploads_active_user
    ON user_avatar_uploads (user_id)
    WHERE status = 'active';

CREATE UNIQUE INDEX uq_user_avatar_uploads_pending_user
    ON user_avatar_uploads (user_id)
    WHERE status = 'pending';

CREATE TRIGGER trg_jamye_set_updated_at BEFORE UPDATE ON user_avatar_uploads
    FOR EACH ROW EXECUTE FUNCTION jamye_set_updated_at_if_changed();
