-- migration: 0002_auth_sessions
-- prerequisite: 0001_core_reliable_messaging.sql
-- reversibility: forward-only; repair or evolve with a new numbered migration
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: separate provider identities from users and keep only refresh-token digests

CREATE TABLE auth_identities (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users (id),
    provider VARCHAR(16) NOT NULL,
    provider_id VARCHAR(128) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT uq_auth_identities_provider_principal UNIQUE (provider, provider_id),
    CONSTRAINT auth_identities_provider_check CHECK (provider IN ('kakao', 'google')),
    CONSTRAINT auth_identities_provider_id_check CHECK (length(provider_id) > 0)
);

CREATE INDEX ix_auth_identities_user
    ON auth_identities (user_id, created_at, id);

CREATE TABLE refresh_sessions (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users (id),
    family_id UUID NOT NULL,
    parent_session_id UUID UNIQUE REFERENCES refresh_sessions (id),
    token_hash BYTEA NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT refresh_sessions_token_hash_check CHECK (octet_length(token_hash) = 32),
    CONSTRAINT refresh_sessions_expiry_check CHECK (expires_at > created_at),
    CONSTRAINT refresh_sessions_consumed_check CHECK (
        consumed_at IS NULL OR consumed_at >= created_at
    ),
    CONSTRAINT refresh_sessions_revoked_check CHECK (
        revoked_at IS NULL OR revoked_at >= created_at
    ),
    CONSTRAINT refresh_sessions_parent_shape_check CHECK (
        parent_session_id IS NULL OR parent_session_id <> id
    )
);

CREATE INDEX ix_refresh_sessions_family
    ON refresh_sessions (family_id, created_at, id);

CREATE INDEX ix_refresh_sessions_user_active
    ON refresh_sessions (user_id, expires_at, id)
    WHERE consumed_at IS NULL AND revoked_at IS NULL;
