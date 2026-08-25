set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter with `nix develop path:.` before running task-4a' >&2; exit 2; fi

# Prove the task-4a behavior target fails only because messaging is not implemented yet.
red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -e src/domain/messaging/mod.rs ]]; then
      printf '%s\n' 'error: RED must run before the task-4a messaging implementation exists' >&2
      exit 2
    fi
    cargo test --locked --test messaging reliable_messaging_surface_is_absent_before_task_4a -- --exact --nocapture

# Prove the text command, idempotency, transaction, delta, auth, and architecture boundaries.
green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    JAMYE_ENABLE_DEV_FIXTURES=true \
      cargo test --locked --test messaging --features dev-fixtures -- \
      --skip 'recovery::' --nocapture
    cargo test --locked --test architecture --all-features

# Prove safe PostgreSQL outage responses, zero partial state, same-key recovery, and log redaction.
postgres-recovery: require-nix-shell
    bash scripts/tasks/task-4a/postgres-recovery.sh
