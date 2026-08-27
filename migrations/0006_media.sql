-- migration: 0006_media
-- prerequisite: 0005_topics.sql
-- reversibility: forward-only; repair or evolve with a new numbered migration
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: add authoritative upload capabilities, ordered message attachments, and one-time topic-media binding
-- lock impact: additive tables/indexes plus a short metadata lock while adding the required topic_media upload binding

CREATE TABLE media_uploads (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users (id),
    object_key VARCHAR(512) NOT NULL,
    scope VARCHAR(8) NOT NULL,
    target_id UUID NOT NULL,
    content_type VARCHAR(64) NOT NULL,
    byte_size BIGINT NOT NULL,
    duration INTEGER,
    filename VARCHAR(255),
    status VARCHAR(16) NOT NULL DEFAULT 'pending',
    bound_message_id UUID REFERENCES messages (id),
    bound_topic_media_id UUID,
    confirmed_at TIMESTAMPTZ,
    consumed_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT uq_media_uploads_object_key UNIQUE (object_key),
    CONSTRAINT uq_media_uploads_bound_topic_media UNIQUE (bound_topic_media_id),
    -- A message may own four attachments, so message identity is not unique here.
    -- Upload identity plus the composite attachment FK makes each upload one-shot.
    CONSTRAINT uq_media_uploads_bound_message_pair UNIQUE (id, bound_message_id),
    CONSTRAINT uq_media_uploads_bound_topic_pair UNIQUE (id, bound_topic_media_id),
    CONSTRAINT fk_media_uploads_bound_topic_media
        FOREIGN KEY (bound_topic_media_id) REFERENCES topic_media (id)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT media_uploads_scope_check CHECK (scope IN ('chat', 'topic')),
    CONSTRAINT media_uploads_object_key_check CHECK (
        length(object_key) BETWEEN 1 AND 512
    ),
    CONSTRAINT media_uploads_content_type_check CHECK (
        length(content_type) BETWEEN 1 AND 64 AND content_type = btrim(content_type)
    ),
    CONSTRAINT media_uploads_byte_size_check CHECK (byte_size > 0),
    CONSTRAINT media_uploads_duration_check CHECK (duration IS NULL OR duration > 0),
    CONSTRAINT media_uploads_filename_check CHECK (
        filename IS NULL OR length(filename) <= 255
    ),
    CONSTRAINT media_uploads_status_check CHECK (
        status IN ('pending', 'confirmed', 'bound', 'expired')
    ),
    CONSTRAINT media_uploads_timestamp_check CHECK (
        expires_at > created_at
        AND (confirmed_at IS NULL OR confirmed_at >= created_at)
        AND (
            consumed_at IS NULL
            OR (confirmed_at IS NOT NULL AND consumed_at >= confirmed_at)
        )
    ),
    CONSTRAINT media_uploads_consumer_shape_check CHECK (
        (
            status = 'pending'
            AND confirmed_at IS NULL
            AND consumed_at IS NULL
            AND bound_message_id IS NULL
            AND bound_topic_media_id IS NULL
        )
        OR (
            status = 'confirmed'
            AND scope = 'chat'
            AND confirmed_at IS NOT NULL
            AND consumed_at IS NULL
            AND bound_message_id IS NULL
            AND bound_topic_media_id IS NULL
        )
        OR (
            status = 'bound'
            AND confirmed_at IS NOT NULL
            AND consumed_at IS NOT NULL
            AND (
                (
                    scope = 'chat'
                    AND bound_message_id IS NOT NULL
                    AND bound_topic_media_id IS NULL
                )
                OR (
                    scope = 'topic'
                    AND bound_message_id IS NULL
                    AND bound_topic_media_id IS NOT NULL
                )
            )
        )
        OR (
            status = 'expired'
            AND consumed_at IS NULL
            AND bound_message_id IS NULL
            AND bound_topic_media_id IS NULL
        )
    )
);

CREATE INDEX ix_media_uploads_user_created
    ON media_uploads (user_id, created_at DESC, id DESC);

CREATE INDEX ix_media_uploads_target_status
    ON media_uploads (scope, target_id, status, expires_at, id);

CREATE INDEX ix_media_uploads_unbound_expiry
    ON media_uploads (expires_at, id)
    WHERE status IN ('pending', 'confirmed');

CREATE TABLE message_media (
    id UUID PRIMARY KEY,
    message_id UUID NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    media_upload_id UUID NOT NULL,
    type VARCHAR(64) NOT NULL,
    object_key VARCHAR(512) NOT NULL,
    width INTEGER,
    height INTEGER,
    byte_size BIGINT NOT NULL,
    duration INTEGER,
    position INTEGER NOT NULL,
    filename VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT uq_message_media_upload UNIQUE (media_upload_id),
    CONSTRAINT uq_message_media_object_key UNIQUE (object_key),
    CONSTRAINT uq_message_media_message_position UNIQUE (message_id, position),
    CONSTRAINT fk_message_media_upload
        FOREIGN KEY (media_upload_id) REFERENCES media_uploads (id),
    CONSTRAINT fk_message_media_bound_upload
        FOREIGN KEY (media_upload_id, message_id)
        REFERENCES media_uploads (id, bound_message_id)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT message_media_type_check CHECK (
        length(type) BETWEEN 1 AND 64 AND type = btrim(type)
    ),
    CONSTRAINT message_media_object_key_check CHECK (
        length(object_key) BETWEEN 1 AND 512
    ),
    CONSTRAINT message_media_width_check CHECK (width IS NULL OR width > 0),
    CONSTRAINT message_media_height_check CHECK (height IS NULL OR height > 0),
    CONSTRAINT message_media_byte_size_check CHECK (byte_size > 0),
    CONSTRAINT message_media_duration_check CHECK (duration IS NULL OR duration > 0),
    CONSTRAINT message_media_position_check CHECK (position BETWEEN 0 AND 3),
    CONSTRAINT message_media_filename_check CHECK (
        filename IS NULL OR length(filename) <= 255
    )
);

ALTER TABLE topic_media
    ADD COLUMN media_upload_id UUID NOT NULL,
    ADD CONSTRAINT uq_topic_media_upload UNIQUE (media_upload_id),
    ADD CONSTRAINT fk_topic_media_upload
        FOREIGN KEY (media_upload_id) REFERENCES media_uploads (id),
    ADD CONSTRAINT fk_topic_media_bound_upload
        FOREIGN KEY (media_upload_id, id)
        REFERENCES media_uploads (id, bound_topic_media_id)
        DEFERRABLE INITIALLY DEFERRED;
