-- migration: 0009_message_anchor_index
-- prerequisite: 0008_account_deletion.sql
-- reversibility: forward-only; additive index, no message or event rewrites
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: bound canonical message-to-event lookup by indexed room and message ID
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '60s';

-- Keep this non-unique: C3 explicitly rejects ambiguous canonical event mappings.
CREATE INDEX ix_conversation_events_message_anchor
    ON conversation_events (conversation_id, (payload ->> 'id')) INCLUDE (cursor)
    WHERE event_type = 'message.created' AND event_version = 1;
