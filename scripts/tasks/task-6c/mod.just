set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter with `nix develop path:.` before running task-6c' >&2; exit 2; fi

# Prove the task-6c target fails only because the revocation surface is absent.
red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -e src/application/realtime/membership_revocation/mod.rs ]]; then
      printf '%s\n' 'error: RED must run before the task-6c membership revocation implementation exists' >&2
      exit 2
    fi
    cargo test --locked --test realtime_membership production_realtime_membership_surface_is_absent_before_task_6c -- --exact --nocapture

# Prove atomic revocation intents, multi-node eviction, and fail-closed delivery authorization.
green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test realtime_membership -- --nocapture
    cargo test --locked --test architecture --all-features
