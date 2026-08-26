set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter with `nix develop path:.` before running task-6b' >&2; exit 2; fi

# Prove the task-6b chatroom/history/read target fails only because task-6b is absent.
red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -e migrations/0004_chatroom_reads.sql ]]; then
      printf '%s\n' 'error: RED must run before the task-6b read-marker migration and implementation exist' >&2
      exit 2
    fi
    cargo test --locked --test chatrooms production_chatrooms_surface_is_absent_before_task_6b -- --exact --nocapture

# Prove membership-safe chatroom/history queries, monotonic reads, migration safety, and wire shape.
green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test chatrooms -- --nocapture
    cargo test --locked --test architecture --all-features
