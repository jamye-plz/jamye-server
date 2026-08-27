set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter with `nix develop path:.` before running task-9' >&2; exit 2; fi

# Prove task-9 starts from locked D9=A with no notification/push migration.
red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -e migrations/0007_notifications_push.sql ]]; then
      printf '%s\n' 'error: RED must run before the task-9 notification/push migration and implementation exist' >&2
      exit 2
    fi
    cargo test --locked --test notifications production_notifications_surface_is_absent_before_task_9 -- --exact --nocapture

# Record a compile-valid RED before the exact-0006 notification/push migration exists.
migration-red: require-nix-shell
    cargo test --locked --test notifications migration::notifications_push_migration_is_forward_only_and_owns_canonical_occurrence_state -- --exact --nocapture

# Verify exact-0006 upgrade, canonical constraints/indexes, and transactional rollback.
migration-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test notifications migration:: -- --nocapture

# Record the absent Task-9 N1/N2/P2-P4 DTO, schema, and fixture contribution.
contract-red: require-nix-shell
    cargo test --locked --test notifications contract:: -- --nocapture

# Verify N1/N2/P2-P4 inventory, D9=A wire shape, and identifier-only handoff.
contract-green: require-nix-shell
    cargo test --locked --test notifications contract:: -- --nocapture

# Record N1 pagination and N2 owner/idempotent single-read orchestration RED.
history-orchestration-red: require-nix-shell
    cargo test --locked --test notifications history_orchestration:: -- --nocapture

# Record P2-P4 validation, secure default, current-owner, and transaction RED.
installation-orchestration-red: require-nix-shell
    cargo test --locked --test notifications installation_orchestration:: -- --nocapture

# Verify N1 pagination and N2 owner/idempotent single-read orchestration.
history-orchestration-green: require-nix-shell
    cargo test --locked --test notifications history_orchestration:: -- --nocapture

# Verify P2-P4 validation, secure default, current-owner, and transaction semantics.
installation-orchestration-green: require-nix-shell
    cargo test --locked --test notifications installation_orchestration:: -- --nocapture

# Record PostgreSQL N1 stable pagination/unread and N2 owner/read-at behavior RED.
history-adapters-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test notifications history_adapters:: -- --nocapture

# Record PostgreSQL P2 convergence/epoch and P3/P4 current-owner behavior RED.
installation-adapters-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test notifications installation_adapters:: -- --nocapture

# Verify PostgreSQL N1 stable pagination/unread and N2 owner/read-at behavior.
history-adapters-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test notifications history_adapters:: -- --nocapture

# Verify PostgreSQL P2 convergence/epoch and P3/P4 current-owner behavior.
installation-adapters-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test notifications installation_adapters:: -- --nocapture

# Record atomic topic/message notification occurrence and bounded-read operations RED.
event-operations-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test notifications event_operations:: -- --nocapture

# Verify atomic topic/message notification occurrences and bounded-read operations.
event-operations-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test notifications event_operations:: -- --nocapture

# Record DB-time send authorization, privacy revalidation, and lock-order RED.
send-authorization-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test notifications send_authorization:: -- --nocapture

# Verify DB-time send authorization, privacy revalidation, and lock ordering.
send-authorization-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test notifications send_authorization:: -- --nocapture

# Record atomic membership/group push invalidation and both lock-order barriers.
privacy-mutations-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test notifications privacy_mutations:: -- --nocapture

# Verify atomic membership/group push invalidation and both lock-order barriers.
privacy-mutations-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test notifications privacy_mutations:: -- --nocapture

# Record push claim/reclaim generation, live-lease completion CAS, and retry state RED.
delivery-lifecycle-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test notifications delivery_lifecycle:: -- --nocapture

# Verify push claim/reclaim generation, live-lease completion CAS, and retry state.
delivery-lifecycle-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test notifications delivery_lifecycle:: -- --nocapture

# Record authorization-first Expo worker ordering, timeout/retry, preview, and invalid-token RED.
delivery-worker-red: require-nix-shell
    cargo test --locked --test notifications delivery_worker:: -- --nocapture

# Verify authorization-first Expo worker ordering, timeout/retry, preview, and invalid-token handling.
delivery-worker-green: require-nix-shell
    cargo test --locked --test notifications delivery_worker:: -- --nocapture

# Record canonical preview lookup and exact invalid-destination PostgreSQL feedback RED.
delivery-adapters-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test notifications delivery_adapters:: -- --nocapture

# Verify canonical preview lookup and exact invalid-destination PostgreSQL feedback.
delivery-adapters-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test notifications delivery_adapters:: -- --nocapture

# Record authenticated N1/N2 and P2-P4 Axum routing/shape/status behavior RED.
http-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test notifications http:: -- --nocapture

# Verify authenticated N1/N2 and P2-P4 Axum routing/shape/status behavior.
http-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test notifications http:: -- --nocapture

# Record Expo HTTP payload, ticket/error classification, and secret-safe logging RED.
expo-adapter-red: require-nix-shell
    cargo test --locked --test notifications expo_adapter:: -- --nocapture

# Verify Expo HTTP payload, ticket/error classification, and secret-safe logging.
expo-adapter-green: require-nix-shell
    cargo test --locked --test notifications expo_adapter:: -- --nocapture

# Record feature-local Expo/lease budget configuration validation RED.
push-config-red: require-nix-shell
    cargo test --locked --test notifications push_config:: -- --nocapture

# Verify feature-local Expo/lease budget configuration validation.
push-config-green: require-nix-shell
    cargo test --locked --test notifications push_config:: -- --nocapture

# Record static PostgreSQL/Expo worker composition and polling runtime RED.
push-runtime-red: require-nix-shell
    cargo test --locked --test notifications push_runtime:: -- --nocapture

# Verify static PostgreSQL/Expo worker composition and polling runtime behavior.
push-runtime-green: require-nix-shell
    cargo test --locked --test notifications push_runtime:: -- --nocapture

# Verify the complete task-9 notification target and existing architecture boundaries.
green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    # A scoped tracing subscriber shares a process-global callsite interest cache.
    # Keep ordinary notification tests parallel, then capture logs in a fresh serial process.
    cargo test --locked --test notifications -- \
      --nocapture \
      --skip expo_adapter::structured_logs_exclude_destination_auth_and_preview_material
    cargo test --locked --test notifications \
      expo_adapter::structured_logs_exclude_destination_auth_and_preview_material -- \
      --exact --nocapture --test-threads=1
    cargo test --locked --test architecture --all-features
