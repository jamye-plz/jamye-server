-- migration: 0008_account_deletion
-- prerequisite: 0007_notifications_push.sql
-- reversibility: forward-only; repair or evolve with a new numbered migration
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: preserve shared authorship through non-authenticating tombstone users
--            and retain deduplicated, generation-fenced cleanup work after account removal
-- lock-impact: additive relations and indexes only; no existing-table rewrite or backfill

-- A tombstone is a deliberately contentless marker for a `users` row that the
-- account-deletion transition creates with a fixed anonymous projection.  It
-- never records the deleted account id or copies profile/credential material.
CREATE TABLE anonymous_author_tombstones (
    user_id UUID PRIMARY KEY REFERENCES users (id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

-- This relation intentionally has no FK to an account, media row, or upload:
-- it must survive the private-row deletion that commits with intent insertion.
-- Object deletion is at-least-once; `object_key` is therefore unique durable
-- work, while provider-side DeleteObject remains idempotent.
CREATE TABLE account_object_deletion_intents (
    id UUID PRIMARY KEY,
    object_key VARCHAR(512) NOT NULL,
    status VARCHAR(16) NOT NULL DEFAULT 'pending',
    claim_owner VARCHAR(128),
    claim_generation BIGINT NOT NULL DEFAULT 0,
    lease_expires_at TIMESTAMPTZ,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ,
    deadline_at TIMESTAMPTZ,
    last_error_code VARCHAR(128),
    succeeded_at TIMESTAMPTZ,
    failed_at TIMESTAMPTZ,
    dead_lettered_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT account_object_deletion_intents_object_key_check CHECK (
        length(object_key) BETWEEN 1 AND 512 AND object_key = btrim(object_key)
    ),
    CONSTRAINT account_object_deletion_intents_status_check CHECK (
        status IN (
            'pending',
            'claimed',
            'retryable',
            'succeeded',
            'failed',
            'dead_letter'
        )
    ),
    CONSTRAINT account_object_deletion_intents_claim_owner_check CHECK (
        claim_owner IS NULL
        OR (
            length(claim_owner) BETWEEN 1 AND 128
            AND claim_owner = btrim(claim_owner)
        )
    ),
    CONSTRAINT account_object_deletion_intents_claim_generation_check CHECK (
        claim_generation >= 0
    ),
    CONSTRAINT account_object_deletion_intents_attempt_count_check CHECK (
        attempt_count >= 0
    ),
    CONSTRAINT account_object_deletion_intents_last_error_code_check CHECK (
        last_error_code IS NULL
        OR (
            length(last_error_code) BETWEEN 1 AND 128
            AND last_error_code = btrim(last_error_code)
        )
    ),
    CONSTRAINT account_object_deletion_intents_claim_state_check CHECK (
        (
            status = 'claimed'
            AND claim_owner IS NOT NULL
            AND lease_expires_at IS NOT NULL
        )
        OR (
            status <> 'claimed'
            AND claim_owner IS NULL
            AND lease_expires_at IS NULL
        )
    ),
    CONSTRAINT account_object_deletion_intents_terminal_state_check CHECK (
        (
            status IN ('pending', 'claimed', 'retryable')
            AND succeeded_at IS NULL
            AND failed_at IS NULL
            AND dead_lettered_at IS NULL
        )
        OR (
            status = 'succeeded'
            AND succeeded_at IS NOT NULL
            AND failed_at IS NULL
            AND dead_lettered_at IS NULL
        )
        OR (
            status = 'failed'
            AND succeeded_at IS NULL
            AND failed_at IS NOT NULL
            AND dead_lettered_at IS NULL
        )
        OR (
            status = 'dead_letter'
            AND succeeded_at IS NULL
            AND failed_at IS NULL
            AND dead_lettered_at IS NOT NULL
        )
    ),
    CONSTRAINT account_object_deletion_intents_terminal_timestamp_check CHECK (
        (succeeded_at IS NULL OR succeeded_at >= created_at)
        AND (failed_at IS NULL OR failed_at >= created_at)
        AND (dead_lettered_at IS NULL OR dead_lettered_at >= created_at)
    )
);

CREATE UNIQUE INDEX ux_account_object_deletion_intents_object_key
    ON account_object_deletion_intents (object_key);

CREATE INDEX ix_account_object_deletion_intents_due
    ON account_object_deletion_intents (
        (COALESCE(next_attempt_at, created_at)),
        created_at,
        id
    )
    WHERE status IN ('pending', 'retryable');

CREATE INDEX ix_account_object_deletion_intents_claim_expiry
    ON account_object_deletion_intents (lease_expires_at, id)
    WHERE status = 'claimed';
