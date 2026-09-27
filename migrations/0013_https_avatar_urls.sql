-- migration: 0013_https_avatar_urls
-- prerequisite: 0012_remove_topic_media.sql
-- reversibility: forward-only; repair or evolve with a new numbered migration
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: normalize stored OAuth profile image URLs to the newly enforced
--   HTTPS-only avatar_url contract without changing NULL or already-HTTPS rows

UPDATE users
SET avatar_url = 'https://' || substring(avatar_url from 8)
WHERE avatar_url LIKE 'http://%';
