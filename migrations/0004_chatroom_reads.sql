-- migration: 0004_chatroom_reads
-- prerequisite: 0003_invites.sql
-- reversibility: forward-only; repair or evolve with a new numbered migration
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: persist one authoritative server-event read cursor per user and chatroom
-- lock impact: additive relation and indexes only; no existing table rewrite or backfill

CREATE TABLE chatroom_reads (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users (id),
    chatroom_id UUID NOT NULL REFERENCES chatrooms (id),
    last_read_cursor BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT uq_chatroom_reads_user_chatroom UNIQUE (user_id, chatroom_id),
    CONSTRAINT chatroom_reads_cursor_check CHECK (last_read_cursor > 0)
);

CREATE INDEX ix_chatroom_reads_chatroom_cursor
    ON chatroom_reads (chatroom_id, last_read_cursor, user_id);
