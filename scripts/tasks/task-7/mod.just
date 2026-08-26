set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter with `nix develop path:.` before running task-7' >&2; exit 2; fi

# Prove the task-7 topics target fails only because the topic migration is absent.
red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -e migrations/0005_topics.sql ]]; then
      printf '%s\n' 'error: RED must run before the task-7 topics migration and implementation exist' >&2
      exit 2
    fi
    cargo test --locked --test topics production_topics_surface_is_absent_before_task_7 -- --exact --nocapture

# Prove topic topology, idempotency, unread projection, tags, migration safety, and wire shape.
green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test topics -- --nocapture
    cargo test --locked --test architecture --all-features
