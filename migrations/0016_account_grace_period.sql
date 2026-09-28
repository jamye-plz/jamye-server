-- migration: 0016_account_grace_period
-- prerequisite: 0015_delete_events_and_live_read_indexes.sql
-- reversibility: forward-only; repair or evolve with a new numbered migration
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: convert account deletion to a restorable grace period with live-row uniqueness and purge leasing
-- lock impact: replaces two unique constraints with live-row partial indexes and adds nullable lifecycle/lease metadata to memberships and users

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '120s';

ALTER TABLE memberships
    ADD COLUMN account_deleted_at TIMESTAMPTZ,
    ADD CONSTRAINT memberships_account_deleted_requires_deleted_check CHECK (
        account_deleted_at IS NULL OR deleted_at IS NOT NULL
    );

ALTER TABLE users
    ADD COLUMN account_purge_claim_owner VARCHAR(128),
    ADD COLUMN account_purge_claim_generation BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN account_purge_claim_expires_at TIMESTAMPTZ,
    ADD CONSTRAINT users_account_purge_claim_owner_check CHECK (
        account_purge_claim_owner IS NULL
        OR (
            length(account_purge_claim_owner) BETWEEN 1 AND 128
            AND account_purge_claim_owner = btrim(account_purge_claim_owner)
        )
    ),
    ADD CONSTRAINT users_account_purge_claim_state_check CHECK (
        (account_purge_claim_owner IS NULL AND account_purge_claim_expires_at IS NULL)
        OR (account_purge_claim_owner IS NOT NULL AND account_purge_claim_expires_at IS NOT NULL)
    ),
    ADD CONSTRAINT users_account_purge_claim_generation_check CHECK (
        account_purge_claim_generation >= 0
    );

ALTER TABLE memberships
    DROP CONSTRAINT uq_memberships_group_user;
CREATE UNIQUE INDEX uq_memberships_group_user
    ON memberships (group_id, user_id)
    WHERE deleted_at IS NULL;

ALTER TABLE auth_identities
    DROP CONSTRAINT uq_auth_identities_provider_principal;
CREATE UNIQUE INDEX uq_auth_identities_provider_principal
    ON auth_identities (provider, provider_id)
    WHERE deleted_at IS NULL;

CREATE INDEX ix_memberships_account_deleted_restore
    ON memberships (user_id, account_deleted_at, group_id)
    WHERE account_deleted_at IS NOT NULL;

CREATE INDEX ix_auth_identities_soft_deleted_provider
    ON auth_identities (provider, provider_id, deleted_at)
    WHERE deleted_at IS NOT NULL;

CREATE INDEX ix_users_account_purge_due
    ON users (deleted_at, account_purge_claim_expires_at, id)
    WHERE deleted_at IS NOT NULL;
