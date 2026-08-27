-- migration: 0007_notifications_push
-- prerequisite: 0006_media.sql
-- reversibility: forward-only; repair or evolve with a new numbered migration
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: add authoritative notification history, Expo installation ownership, and durable per-source-event push occurrences
-- lock impact: additive relations and indexes only; no existing table rewrite or backfill

CREATE TABLE notifications (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users (id),
    topic_id UUID REFERENCES topics (id),
    conversation_id UUID REFERENCES chatrooms (id),
    source_cursor BIGINT,
    type VARCHAR(32) NOT NULL,
    payload JSONB NOT NULL,
    dedup_key VARCHAR(256),
    read_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT notifications_type_check CHECK (
        type IN ('new_topic', 'chat_unread', 'other')
    ),
    CONSTRAINT notifications_payload_check CHECK (
        jsonb_typeof(payload) = 'object'
    ),
    CONSTRAINT notifications_source_cursor_check CHECK (
        source_cursor IS NULL OR source_cursor > 0
    ),
    CONSTRAINT notifications_dedup_key_check CHECK (
        dedup_key IS NULL
        OR (
            length(dedup_key) BETWEEN 1 AND 256
            AND dedup_key = btrim(dedup_key)
        )
    ),
    CONSTRAINT notifications_topic_shape_check CHECK (
        type = 'other'
        OR (
            topic_id IS NOT NULL
            AND conversation_id IS NOT NULL
            AND source_cursor IS NOT NULL
            AND dedup_key IS NOT NULL
        )
    ),
    CONSTRAINT notifications_read_timestamp_check CHECK (
        read_at IS NULL OR read_at >= created_at
    )
);

CREATE UNIQUE INDEX ux_notifications_user_dedup
    ON notifications (user_id, dedup_key)
    WHERE dedup_key IS NOT NULL;

CREATE INDEX ix_notifications_user_created
    ON notifications (user_id, created_at DESC, id DESC);

CREATE INDEX ix_notifications_user_unread
    ON notifications (user_id, created_at DESC, id DESC)
    WHERE read_at IS NULL;

CREATE INDEX ix_notifications_topic_cursor
    ON notifications (user_id, topic_id, source_cursor, id)
    WHERE topic_id IS NOT NULL AND source_cursor IS NOT NULL;

CREATE TABLE push_installations (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users (id),
    owner_epoch BIGINT NOT NULL DEFAULT 1,
    installation_id VARCHAR(255) NOT NULL,
    platform VARCHAR(8) NOT NULL,
    provider VARCHAR(8) NOT NULL DEFAULT 'expo',
    token VARCHAR(512) NOT NULL,
    environment VARCHAR(16) NOT NULL,
    message_preview_enabled BOOLEAN NOT NULL DEFAULT false,
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    disabled_at TIMESTAMPTZ,
    CONSTRAINT uq_push_installations_installation_id UNIQUE (installation_id),
    CONSTRAINT uq_push_installations_destination UNIQUE (environment, token),
    CONSTRAINT push_installations_owner_epoch_check CHECK (owner_epoch > 0),
    CONSTRAINT push_installations_installation_id_check CHECK (
        length(installation_id) BETWEEN 1 AND 255
        AND installation_id = btrim(installation_id)
    ),
    CONSTRAINT push_installations_platform_check CHECK (
        platform IN ('ios', 'android')
    ),
    CONSTRAINT push_installations_provider_check CHECK (provider = 'expo'),
    CONSTRAINT push_installations_token_check CHECK (
        length(token) BETWEEN 1 AND 512 AND token = btrim(token)
    ),
    CONSTRAINT push_installations_environment_check CHECK (
        environment IN ('development', 'production')
    ),
    CONSTRAINT push_installations_disabled_timestamp_check CHECK (
        disabled_at IS NULL OR disabled_at >= last_seen_at
    )
);

CREATE INDEX ix_push_installations_user
    ON push_installations (user_id, disabled_at, last_seen_at DESC, id);

CREATE TABLE push_delivery_intents (
    id UUID PRIMARY KEY,
    notification_id UUID NOT NULL REFERENCES notifications (id),
    source_event_id UUID NOT NULL REFERENCES conversation_events (id),
    source_message_id UUID REFERENCES messages (id),
    recipient_user_id UUID NOT NULL REFERENCES users (id),
    push_installation_id UUID NOT NULL REFERENCES push_installations (id),
    installation_owner_epoch BIGINT NOT NULL,
    message_preview_enabled_snapshot BOOLEAN NOT NULL,
    provider VARCHAR(8) NOT NULL DEFAULT 'expo',
    payload JSONB NOT NULL,
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
    CONSTRAINT uq_push_delivery_source_installation
        UNIQUE (source_event_id, push_installation_id),
    CONSTRAINT push_delivery_owner_epoch_check CHECK (
        installation_owner_epoch > 0
    ),
    CONSTRAINT push_delivery_provider_check CHECK (provider = 'expo'),
    CONSTRAINT push_delivery_payload_check CHECK (
        jsonb_typeof(payload) = 'object'
    ),
    CONSTRAINT push_delivery_status_check CHECK (
        status IN (
            'pending',
            'claimed',
            'retryable',
            'succeeded',
            'failed',
            'dead_letter'
        )
    ),
    CONSTRAINT push_delivery_claim_owner_check CHECK (
        claim_owner IS NULL OR length(claim_owner) > 0
    ),
    CONSTRAINT push_delivery_claim_generation_check CHECK (
        claim_generation >= 0
    ),
    CONSTRAINT push_delivery_attempt_count_check CHECK (attempt_count >= 0),
    CONSTRAINT push_delivery_last_error_code_check CHECK (
        last_error_code IS NULL OR length(last_error_code) > 0
    ),
    CONSTRAINT push_delivery_claim_state_check CHECK (
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
    CONSTRAINT push_delivery_terminal_state_check CHECK (
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
    CONSTRAINT push_delivery_terminal_timestamp_check CHECK (
        (succeeded_at IS NULL OR succeeded_at >= created_at)
        AND (failed_at IS NULL OR failed_at >= created_at)
        AND (dead_lettered_at IS NULL OR dead_lettered_at >= created_at)
    )
);

CREATE INDEX ix_push_delivery_due
    ON push_delivery_intents (
        (COALESCE(next_attempt_at, created_at)),
        created_at,
        id
    )
    WHERE status IN ('pending', 'retryable');

CREATE INDEX ix_push_delivery_claim_expiry
    ON push_delivery_intents (lease_expires_at, id)
    WHERE status = 'claimed';

CREATE INDEX ix_push_delivery_recipient
    ON push_delivery_intents (recipient_user_id, status, created_at, id);

CREATE INDEX ix_push_delivery_installation_epoch
    ON push_delivery_intents (
        push_installation_id,
        installation_owner_epoch,
        status,
        id
    );
