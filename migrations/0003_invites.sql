-- migration: 0003_invites
-- prerequisite: 0002_auth_sessions.sql
-- reversibility: forward-only; repair or evolve with a new numbered migration
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: add bounded group invitations without moving membership authority out of PostgreSQL

CREATE TABLE invites (
    id UUID PRIMARY KEY,
    group_id UUID NOT NULL REFERENCES groups (id),
    code VARCHAR(64) NOT NULL,
    created_by UUID NOT NULL REFERENCES users (id),
    expires_at TIMESTAMPTZ,
    max_uses INTEGER,
    used_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT uq_invites_code UNIQUE (code),
    CONSTRAINT invites_code_length_check CHECK (length(code) BETWEEN 16 AND 64),
    CONSTRAINT invites_max_uses_check CHECK (max_uses IS NULL OR max_uses > 0),
    CONSTRAINT invites_used_count_check CHECK (used_count >= 0),
    CONSTRAINT invites_usage_bound_check CHECK (
        max_uses IS NULL OR used_count <= max_uses
    )
);

CREATE INDEX ix_invites_group_created
    ON invites (group_id, created_at, id);

CREATE INDEX ix_invites_creator_created
    ON invites (created_by, created_at, id);
