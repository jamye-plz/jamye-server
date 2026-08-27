set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter with `nix develop path:.` before running task-8' >&2; exit 2; fi

# Prove task-8 starts from the selected D11=B boundary with no media migration.
red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -e migrations/0006_media.sql ]]; then
      printf '%s\n' 'error: RED must run before the task-8 media migration and implementation exist' >&2
      exit 2
    fi
    cargo test --locked --test media production_media_surface_is_absent_before_task_8 -- --exact --nocapture

# Regenerate and verify Cargo/Nix locks after the bounded task-8 dependency delta.
locks: require-nix-shell
    just task-1 locks-create
    just task-1 locks-verify

# Reuse the repository-wide advisory, ban, license, and source policy.
dependency-check: require-nix-shell
    just task-1 dependency-check

# Record a compiling behavior RED for the selected D11=B bucket lifecycle.
bucket-red: require-nix-shell
    cargo test --locked --test media bucket_lifecycle:: -- --nocapture

# Verify the selected D11=B bucket lifecycle after implementation.
bucket-green: require-nix-shell
    cargo test --locked --test media bucket_lifecycle:: -- --nocapture

# Record a compiling RED for feature-local fail-closed object-storage config.
config-red: require-nix-shell
    cargo test --locked --test media config:: -- --nocapture

# Verify feature-local fail-closed object-storage config after implementation.
config-green: require-nix-shell
    cargo test --locked --test media config:: -- --nocapture

# Record a compiling RED for the concrete AWS SDK -> S3-compatible HTTP boundary.
sdk-red: require-nix-shell
    cargo test --locked --test media s3_bucket_backend:: -- --nocapture

# Verify signed path-style HEAD/create and typed provider classification.
sdk-green: require-nix-shell
    cargo test --locked --test media s3_bucket_backend:: -- --nocapture

# Record a compile-valid RED before the forward-only media schema exists.
migration-red: require-nix-shell
    cargo test --locked --test media migration::media_migration_is_forward_only_and_owns_one_time_binding_constraints -- --exact --nocapture

# Verify exact-0005 upgrade, rollback, and one-time ordered binding constraints.
migration-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test media migration:: -- --nocapture

# Record a service-free RED for the policy shared by presign and finalize.
policy-red: require-nix-shell
    cargo test --locked --test media policy:: -- --nocapture

# Verify exact MIME/cap/filename/scope/key policy after implementation.
policy-green: require-nix-shell
    cargo test --locked --test media policy:: -- --nocapture

# Record a service-free RED for MD1 upload-intent orchestration and rollback.
upload-intent-red: require-nix-shell
    cargo test --locked --test media upload_intent:: -- --nocapture

# Verify policy/rate-limit/transaction/repository/presign ordering after implementation.
upload-intent-green: require-nix-shell
    cargo test --locked --test media upload_intent:: -- --nocapture

# Record a RED for real PostgreSQL target authorization/insert and SDK PUT presigning.
upload-intent-adapters-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test media upload_intent_adapters:: -- --nocapture

# Verify the concrete PostgreSQL and AWS SDK upload-intent adapters.
upload-intent-adapters-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test media upload_intent_adapters:: -- --nocapture

# Record a service-free RED for authoritative object metadata and audio duration.
finalize-policy-red: require-nix-shell
    cargo test --locked --test media finalize_policy:: -- --nocapture

# Verify exact HEAD metadata and fail-closed audio-duration policy.
finalize-policy-green: require-nix-shell
    cargo test --locked --test media finalize_policy:: -- --nocapture

# Record a service-free RED for MD2 authorization, inspection, transaction, and retry ordering.
finalize-orchestration-red: require-nix-shell
    cargo test --locked --test media finalize_orchestration:: -- --nocapture

# Verify MD2 authorization, inspection, atomic topic promotion, rollback, and exact retry.
finalize-orchestration-green: require-nix-shell
    cargo test --locked --test media finalize_orchestration:: -- --nocapture

# Record a RED for PostgreSQL finalize persistence and signed S3 HEAD/audio probing.
finalize-adapters-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test media finalize_adapters:: -- --nocapture

# Verify caller-owned finalize persistence and authoritative SDK/Symphonia inspection.
finalize-adapters-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test media finalize_adapters:: -- --nocapture

# Record a service-free RED for C4 message/media composition and voice invariants.
message-policy-red: require-nix-shell
    cargo test --locked --test media message_policy:: -- --nocapture

# Verify ordered 0..3 visual media and bodyless exactly-one-audio voice policy.
message-policy-green: require-nix-shell
    cargo test --locked --test media message_policy:: -- --nocapture

# Record a RED for caller-owned PostgreSQL message-media binding and exact retry.
message-binding-adapters-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test media message_binding_adapters:: -- --nocapture

# Verify ordered capability consumption, BOLA guards, rollback, and canonical retry.
message-binding-adapters-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test media message_binding_adapters:: -- --nocapture

# Record a service-free RED for MD5 safe UTF-8 download Content-Disposition.
access-policy-red: require-nix-shell
    cargo test --locked --test media access_policy:: -- --nocapture

# Verify RFC 5987 filename*, sanitized fallback, and MIME-based fallback names.
access-policy-green: require-nix-shell
    cargo test --locked --test media access_policy:: -- --nocapture

# Record a service-free RED for authorization-before-presign MD4/MD5 orchestration.
access-orchestration-red: require-nix-shell
    cargo test --locked --test media access_orchestration:: -- --nocapture

# Verify short GET TTL, BOLA stop order, safe download override, and storage-only errors.
access-orchestration-green: require-nix-shell
    cargo test --locked --test media access_orchestration:: -- --nocapture

# Record a RED for concrete PostgreSQL BOLA lookup and public-origin SDK GET presigning.
access-adapters-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test media access_adapters:: -- --nocapture

# Verify message/topic membership lookup and view/download SDK GET signatures.
access-adapters-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test media access_adapters:: -- --nocapture

# Record a compile-valid RED for the authenticated MD4/MD5 HTTP boundary.
access-http-red: require-nix-shell
    cargo test --locked --test media access_http:: -- --nocapture

# Verify public-safe MD4 JSON, MD5 redirect, UUID validation, and stable errors.
access-http-green: require-nix-shell
    cargo test --locked --test media access_http:: -- --nocapture

# Record a service-free RED for authenticated MD1 intent and MD2 finalize HTTP contracts.
upload-finalize-http-red: require-nix-shell
    cargo test --locked --test media upload_finalize_http:: -- --nocapture

# Verify MD1/MD2 public projections, request validation, and stable HTTP errors.
upload-finalize-http-green: require-nix-shell
    cargo test --locked --test media upload_finalize_http:: -- --nocapture

# Record a PostgreSQL RED for ordered history media and MD3 upload identity projection.
read-projections-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test media read_projections:: -- --nocapture

# Verify ordered public history attachments and canonical topic media_upload_id.
read-projections-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test media read_projections:: -- --nocapture

# Record the missing authenticated MD3 paginated collection as a PostgreSQL-backed RED.
md3-http-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test media md3_http:: -- --nocapture

# Verify the authenticated, membership-safe MD3 topic-media collection.
md3-http-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test media md3_http:: -- --nocapture

# Record the absent/stale Task-8 media operation, wire-schema, and fixture contributions.
contract-red: require-nix-shell
    cargo test --locked --test media contract:: -- --nocapture

# Verify MD1-MD5 inventory plus ordered history/topic/voice media contract contributions.
contract-green: require-nix-shell
    cargo test --locked --test media contract:: -- --nocapture

# Record the disposable MinIO app identity's missing least-privilege policy as RED.
minio-boundary-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    require_nix_command curl
    if [[ "${JAMYE_ENVIRONMENT:-}" != "test" ]] \
      || [[ "${JAMYE_MINIO_HEALTH_URL:-}" != "http://127.0.0.1:9000/minio/health/live" ]]; then
      printf '%s\n' 'error: MinIO boundary RED accepts only the task-1 loopback test environment' >&2
      exit 2
    fi
    curl --fail --silent --show-error --max-time 3 "$JAMYE_MINIO_HEALTH_URL" >/dev/null
    cargo test --locked --test media minio_boundary:: -- --nocapture

# Apply the exact disposable app policy and verify app-owned private object flow.
minio-boundary-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    bash scripts/tasks/task-8/apply-minio-policy.sh
    cargo test --locked --test media minio_boundary:: -- --nocapture

# Verify storage-only degradation and JSON-log secret non-disclosure as one composite gate.
resilience-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    JAMYE_ENABLE_DEV_FIXTURES=true \
      cargo test --locked --test media --features dev-fixtures \
      media_resilience_ -- --nocapture --test-threads=1

# Verify the complete task-8 media target and existing architecture boundaries.
green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    bash scripts/tasks/task-8/apply-minio-policy.sh

    # Scoped tracing subscribers share a process-global callsite interest cache.
    # Keep ordinary media tests parallel, then capture logs in a fresh serial process.
    JAMYE_ENABLE_DEV_FIXTURES=true \
      cargo test --locked --test media --features dev-fixtures -- \
      --nocapture --skip media_resilience_
    JAMYE_ENABLE_DEV_FIXTURES=true \
      cargo test --locked --test media --features dev-fixtures \
      media_resilience_ -- --nocapture --test-threads=1
    cargo test --locked --test architecture --all-features
