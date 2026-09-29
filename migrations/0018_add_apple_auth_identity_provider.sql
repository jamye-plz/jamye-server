-- migration: 0018_add_apple_auth_identity_provider
-- prerequisite: 0017_remaining_soft_delete_live_uniqueness.sql
-- reversibility: forward-only; repair or evolve with a new numbered migration
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: allow Sign in with Apple identities while preserving the existing auth identity table and provider-token non-storage invariant
-- lock impact: rewrites one CHECK constraint on auth_identities under lock_timeout/statement_timeout; no table, column, index, or row rewrite is performed

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '120s';

ALTER TABLE auth_identities
    DROP CONSTRAINT auth_identities_provider_check,
    ADD CONSTRAINT auth_identities_provider_check
        CHECK (provider IN ('kakao', 'google', 'apple'));
