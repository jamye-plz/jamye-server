-- migration: 0005_topics
-- prerequisite: 0004_chatroom_reads.sql
-- reversibility: forward-only; repair or evolve with a new numbered migration
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: add authoritative topic, tag, and base topic-media state plus the deferred chatroom FK
-- lock impact: additive relations/indexes, then one short metadata lock to add chatrooms.topic_id FK

CREATE TABLE topics (
    id UUID PRIMARY KEY,
    group_id UUID NOT NULL REFERENCES groups (id),
    author_id UUID NOT NULL REFERENCES users (id),
    idempotency_key UUID NOT NULL,
    request_fingerprint CHAR(64) NOT NULL,
    title VARCHAR(256) NOT NULL,
    body TEXT,
    status VARCHAR(16) NOT NULL DEFAULT 'seed',
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT uq_topics_author_idempotency UNIQUE (author_id, idempotency_key),
    CONSTRAINT topics_request_fingerprint_check CHECK (
        request_fingerprint ~ '^[0-9a-f]{64}$'
    ),
    CONSTRAINT topics_title_check CHECK (
        length(title) BETWEEN 1 AND 256 AND title = btrim(title)
    ),
    CONSTRAINT topics_body_check CHECK (body IS NULL OR length(body) > 0),
    CONSTRAINT topics_status_check CHECK (status IN ('seed', 'enriched')),
    CONSTRAINT topics_timestamp_check CHECK (updated_at >= created_at)
);

CREATE INDEX ix_topics_group_created
    ON topics (group_id, created_at DESC, id DESC);

CREATE INDEX ix_topics_author_created
    ON topics (author_id, created_at DESC, id DESC);

CREATE TABLE topic_media (
    id UUID PRIMARY KEY,
    topic_id UUID NOT NULL REFERENCES topics (id) ON DELETE CASCADE,
    type VARCHAR(64) NOT NULL,
    object_key VARCHAR(512) NOT NULL,
    width INTEGER,
    height INTEGER,
    byte_size BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT uq_topic_media_topic_object UNIQUE (topic_id, object_key),
    CONSTRAINT topic_media_type_check CHECK (length(type) > 0),
    CONSTRAINT topic_media_object_key_check CHECK (length(object_key) > 0),
    CONSTRAINT topic_media_width_check CHECK (width IS NULL OR width > 0),
    CONSTRAINT topic_media_height_check CHECK (height IS NULL OR height > 0),
    CONSTRAINT topic_media_byte_size_check CHECK (byte_size IS NULL OR byte_size > 0)
);

CREATE INDEX ix_topic_media_topic_created
    ON topic_media (topic_id, created_at, id);

CREATE TABLE topic_tags (
    id UUID PRIMARY KEY,
    topic_id UUID NOT NULL REFERENCES topics (id) ON DELETE CASCADE,
    tag VARCHAR(64) NOT NULL,
    source VARCHAR(8) NOT NULL,
    confidence DOUBLE PRECISION,
    CONSTRAINT uq_topic_tags_topic_tag UNIQUE (topic_id, tag),
    CONSTRAINT topic_tags_tag_check CHECK (
        length(tag) BETWEEN 1 AND 64 AND tag = btrim(tag)
    ),
    CONSTRAINT topic_tags_source_check CHECK (source IN ('ai', 'user')),
    CONSTRAINT topic_tags_confidence_check CHECK (
        confidence IS NULL OR (confidence >= 0.0 AND confidence <= 1.0)
    )
);

CREATE INDEX ix_topic_tags_topic_tag
    ON topic_tags (topic_id, tag, id);

-- 0001 deliberately deferred this reference until the topics table existed.
ALTER TABLE chatrooms
    ADD CONSTRAINT fk_chatrooms_topic_id
    FOREIGN KEY (topic_id) REFERENCES topics (id);
