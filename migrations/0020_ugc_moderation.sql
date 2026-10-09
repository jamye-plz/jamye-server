-- migration: 0020_ugc_moderation
-- prerequisite: 0019_user_avatar_uploads.sql
-- reversibility: forward-only; repair or evolve with a new numbered migration
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: add the user-generated-content moderation core (user blocks, message and user reports with a message snapshot, account suspension) and let the existing push pipeline carry a content-free operator alert for a stored report
-- lock impact: two new relations and their indexes (including the report target lookups used by account purge and operator actioning); two nullable users columns (metadata-only, no rewrite); push_delivery_intents gains one nullable column and two relaxed NOT NULL constraints that existing rows already satisfy, so no existing row is rewritten or backfilled. Locks are bounded by lock_timeout/statement_timeout and migrate runs before the api and worker start

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '120s';

-- Suspension is a nullable pair on the account row. Clearing suspended_at unsuspends;
-- the reason never exists without the timestamp.
ALTER TABLE users
    ADD COLUMN suspended_at TIMESTAMPTZ,
    ADD COLUMN suspension_reason TEXT,
    ADD CONSTRAINT users_suspension_reason_check CHECK (
        suspension_reason IS NULL
        OR (suspended_at IS NOT NULL AND length(suspension_reason) BETWEEN 1 AND 500)
    );

-- One-directional block. A block never changes membership or message delivery; it only
-- hides the blocked user for the blocker (client side) and suppresses the blocker's pushes.
CREATE TABLE user_blocks (
    blocker_id UUID NOT NULL REFERENCES users (id),
    blocked_id UUID NOT NULL REFERENCES users (id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT user_blocks_pkey PRIMARY KEY (blocker_id, blocked_id),
    CONSTRAINT user_blocks_not_self_check CHECK (blocker_id <> blocked_id)
);

CREATE INDEX ix_user_blocks_blocked
    ON user_blocks (blocked_id);

-- A report targets exactly one message or one user. The message snapshot keeps the
-- already-masked text and media references so a moderator can act after the author deletes
-- the message. Reports are retained when an account is purged (their user references are
-- reassigned to the anonymous tombstone by the purge).
CREATE TABLE reports (
    id UUID PRIMARY KEY,
    reporter_id UUID NOT NULL REFERENCES users (id),
    target_type VARCHAR(8) NOT NULL,
    target_message_id UUID REFERENCES messages (id),
    target_user_id UUID REFERENCES users (id),
    target_group_id UUID NOT NULL REFERENCES groups (id),
    reason VARCHAR(16) NOT NULL,
    message_snapshot JSONB,
    status VARCHAR(16) NOT NULL DEFAULT 'open',
    handled_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT reports_target_type_check CHECK (target_type IN ('message', 'user')),
    CONSTRAINT reports_reason_check CHECK (
        reason IN (
            'spam',
            'harassment',
            'sexual_content',
            'violence',
            'hate',
            'illegal',
            'other'
        )
    ),
    CONSTRAINT reports_status_check CHECK (status IN ('open', 'actioned', 'dismissed')),
    CONSTRAINT reports_exactly_one_target_check CHECK (
        (
            target_type = 'message'
            AND target_message_id IS NOT NULL
            AND target_user_id IS NULL
            AND message_snapshot IS NOT NULL
            AND jsonb_typeof(message_snapshot) = 'object'
        )
        OR (
            target_type = 'user'
            AND target_user_id IS NOT NULL
            AND target_message_id IS NULL
            AND message_snapshot IS NULL
        )
    ),
    CONSTRAINT reports_handled_state_check CHECK (
        (status = 'open' AND handled_at IS NULL)
        OR (status <> 'open' AND handled_at IS NOT NULL)
    )
);

CREATE INDEX ix_reports_status_created
    ON reports (status, created_at, id);

CREATE INDEX ix_reports_reporter_created
    ON reports (reporter_id, created_at);

-- Account purge reassigns reports by target user, and the operator CLI actions the open
-- reports of a message; both are equality lookups on these columns.
CREATE INDEX ix_reports_target_user
    ON reports (target_user_id);

CREATE INDEX ix_reports_target_message
    ON reports (target_message_id);

-- The operator alert is a push occurrence that belongs to a report instead of a
-- notification and conversation event. Conversation occurrences keep both references
-- (the check below forces exactly one of the two shapes), and an alert is unique per
-- report and installation.
ALTER TABLE push_delivery_intents
    ALTER COLUMN notification_id DROP NOT NULL,
    ALTER COLUMN source_event_id DROP NOT NULL,
    ADD COLUMN report_id UUID REFERENCES reports (id),
    ADD CONSTRAINT push_delivery_subject_check CHECK (
        (
            report_id IS NULL
            AND notification_id IS NOT NULL
            AND source_event_id IS NOT NULL
        )
        OR (
            report_id IS NOT NULL
            AND notification_id IS NULL
            AND source_event_id IS NULL
            AND source_message_id IS NULL
        )
    );

CREATE UNIQUE INDEX uq_push_delivery_report_installation
    ON push_delivery_intents (report_id, push_installation_id)
    WHERE report_id IS NOT NULL AND deleted_at IS NULL;
