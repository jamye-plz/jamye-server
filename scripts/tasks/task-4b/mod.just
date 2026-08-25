set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter with `nix develop path:.` before running task-4b' >&2; exit 2; fi

# Prove the task-4b target fails only because the realtime delivery surface is absent.
red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -e src/application/realtime/mod.rs ]]; then
      printf '%s\n' 'error: RED must run before the task-4b realtime implementation exists' >&2
      exit 2
    fi
    cargo test --locked --test realtime realtime_surface_is_absent_before_task_4b -- --exact --nocapture

# Regenerate and verify Cargo/Nix locks after the bounded realtime dependency delta.
locks: require-nix-shell
    just task-1 locks-create
    just task-1 locks-verify

# Reuse the repository-wide advisory, ban, license, and source policy.
dependency-check: require-nix-shell
    just task-1 dependency-check

# Prove outbox leasing, Redis ticket/PubSub, WebSocket lifecycle, C1, and architecture boundaries.
green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    JAMYE_ENABLE_DEV_FIXTURES=true \
      cargo test --locked --test realtime --features dev-fixtures -- \
      --skip 'redis_recovery::' --nocapture
    cargo test --locked --test architecture --all-features

# Stop/restart only guarded local Redis and prove durable retry plus same-router recovery.
redis-recovery: require-nix-shell
    bash scripts/tasks/task-4b/redis-recovery.sh
