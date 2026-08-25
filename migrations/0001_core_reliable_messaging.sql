-- migration: 0001_core_reliable_messaging
-- prerequisite: empty PostgreSQL database
-- reversibility: forward-only; repair or evolve with a new numbered migration
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: establish the minimal authoritative group, message, event, and outbox state

CREATE TABLE users (
    id UUID PRIMARY KEY,
    nickname VARCHAR(64) NOT NULL,
    avatar_url VARCHAR(512),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE TABLE groups (
    id UUID PRIMARY KEY,
    name VARCHAR(128) NOT NULL,
    owner_id UUID NOT NULL REFERENCES users (id),
    max_members INTEGER NOT NULL DEFAULT 12,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    deleted_at TIMESTAMPTZ,
    CONSTRAINT groups_max_members_check CHECK (max_members > 0)
);

CREATE TABLE memberships (
    id UUID PRIMARY KEY,
    group_id UUID NOT NULL REFERENCES groups (id),
    user_id UUID NOT NULL REFERENCES users (id),
    role VARCHAR(16) NOT NULL,
    joined_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT uq_memberships_group_user UNIQUE (group_id, user_id),
    CONSTRAINT memberships_role_check CHECK (role IN ('owner', 'member'))
);

CREATE TABLE chatrooms (
    id UUID PRIMARY KEY,
    group_id UUID NOT NULL REFERENCES groups (id),
    type VARCHAR(8) NOT NULL,
    topic_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT chatrooms_type_topic_check CHECK (
        (type = 'main' AND topic_id IS NULL)
        OR (type = 'topic' AND topic_id IS NOT NULL)
    )
);

-- A topics relation does not exist until 0005. The type/shape and uniqueness
-- invariants are safe to establish now; 0005 alone adds the topic_id FK.
CREATE UNIQUE INDEX ux_chatrooms_one_main_per_group
    ON chatrooms (group_id)
    WHERE type = 'main';

CREATE UNIQUE INDEX ux_chatrooms_one_topic_per_topic
    ON chatrooms (topic_id)
    WHERE type = 'topic';

CREATE TABLE messages (
    id UUID PRIMARY KEY,
    chatroom_id UUID NOT NULL REFERENCES chatrooms (id),
    sender_id UUID REFERENCES users (id),
    client_msg_id UUID,
    body TEXT,
    type VARCHAR(8) NOT NULL DEFAULT 'user',
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT messages_type_check CHECK (type IN ('user', 'system')),
    CONSTRAINT messages_identity_check CHECK (
        (type = 'user' AND sender_id IS NOT NULL AND client_msg_id IS NOT NULL)
        OR (type = 'system' AND sender_id IS NULL AND client_msg_id IS NULL)
    )
);

CREATE UNIQUE INDEX ux_messages_sender_client_msg_id
    ON messages (sender_id, client_msg_id)
    WHERE client_msg_id IS NOT NULL;

CREATE INDEX ix_messages_chatroom_created
    ON messages (chatroom_id, created_at, id);

CREATE TABLE conversation_events (
    id UUID PRIMARY KEY,
    cursor BIGINT GENERATED ALWAYS AS IDENTITY,
    conversation_id UUID NOT NULL,
    event_type VARCHAR(128) NOT NULL,
    event_version SMALLINT NOT NULL,
    payload JSONB NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT uq_conversation_events_cursor UNIQUE (cursor),
    CONSTRAINT conversation_events_type_check CHECK (length(event_type) > 0),
    CONSTRAINT conversation_events_version_check CHECK (event_version > 0),
    CONSTRAINT conversation_events_payload_check CHECK (jsonb_typeof(payload) = 'object')
);

CREATE INDEX ix_conversation_events_conversation_cursor
    ON conversation_events (conversation_id, cursor);

CREATE TABLE outbox_events (
    id UUID PRIMARY KEY,
    intent_type VARCHAR(16) NOT NULL,
    event_type VARCHAR(128) NOT NULL,
    event_version SMALLINT NOT NULL,
    aggregate_type VARCHAR(16) NOT NULL,
    aggregate_id UUID NOT NULL,
    conversation_event_id UUID REFERENCES conversation_events (id),
    payload JSONB NOT NULL,
    status VARCHAR(16) NOT NULL DEFAULT 'pending',
    claim_owner VARCHAR(128),
    claim_generation BIGINT NOT NULL DEFAULT 0,
    claim_expires_at TIMESTAMPTZ,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ,
    deadline_at TIMESTAMPTZ,
    last_error_code VARCHAR(128),
    published_at TIMESTAMPTZ,
    dead_lettered_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT outbox_intent_type_check CHECK (intent_type IN ('conversation', 'control')),
    CONSTRAINT outbox_event_type_check CHECK (length(event_type) > 0),
    CONSTRAINT outbox_event_version_check CHECK (event_version > 0),
    CONSTRAINT outbox_aggregate_type_check CHECK (
        aggregate_type IN ('conversation', 'membership', 'group')
    ),
    CONSTRAINT outbox_payload_check CHECK (jsonb_typeof(payload) = 'object'),
    CONSTRAINT outbox_status_check CHECK (
        status IN ('pending', 'claimed', 'published', 'dead_letter')
    ),
    CONSTRAINT outbox_claim_owner_check CHECK (
        claim_owner IS NULL OR length(claim_owner) > 0
    ),
    CONSTRAINT outbox_claim_generation_check CHECK (claim_generation >= 0),
    CONSTRAINT outbox_attempt_count_check CHECK (attempt_count >= 0),
    CONSTRAINT outbox_claim_state_check CHECK (
        (
            status = 'claimed'
            AND claim_owner IS NOT NULL
            AND claim_expires_at IS NOT NULL
        )
        OR (
            status <> 'claimed'
            AND claim_owner IS NULL
            AND claim_expires_at IS NULL
        )
    ),
    CONSTRAINT outbox_intent_shape_check CHECK (
        (
            intent_type = 'conversation'
            AND aggregate_type = 'conversation'
            AND conversation_event_id IS NOT NULL
        )
        OR (
            intent_type = 'control'
            AND aggregate_type IN ('membership', 'group')
            AND conversation_event_id IS NULL
        )
    ),
    CONSTRAINT outbox_terminal_state_check CHECK (
        (
            status IN ('pending', 'claimed')
            AND published_at IS NULL
            AND dead_lettered_at IS NULL
        )
        OR (
            status = 'published'
            AND published_at IS NOT NULL
            AND dead_lettered_at IS NULL
        )
        OR (
            status = 'dead_letter'
            AND published_at IS NULL
            AND dead_lettered_at IS NOT NULL
        )
    )
);

CREATE INDEX ix_outbox_events_pending_due
    ON outbox_events (
        (COALESCE(next_attempt_at, created_at)),
        created_at,
        id
    )
    WHERE status = 'pending';

CREATE INDEX ix_outbox_events_claim_expiry
    ON outbox_events (claim_expires_at, id)
    WHERE status = 'claimed';
