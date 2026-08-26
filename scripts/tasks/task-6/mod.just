set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter with `nix develop path:.` before running task-6' >&2; exit 2; fi

# Prove the task-6 group and invite target fails only because task-6 is absent.
red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -e migrations/0003_invites.sql ]]; then
      printf '%s\n' 'error: RED must run before the task-6 invite migration and implementation exist' >&2
      exit 2
    fi
    cargo test --locked --test groups production_groups_surface_is_absent_before_task_6 -- --exact --nocapture

# Prove group topology, ownership, bounded invites, migration safety, HTTP wire, and shared limits.
green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    cargo test --locked --test groups -- \
      --skip redis_recovery::redis_stop_restart_preserves_invite_authority_and_recovers_the_same_limiter \
      --nocapture
    cargo test --locked --test architecture --all-features

# Stop/restart only guarded local Redis and prove PostgreSQL group authority is preserved.
redis-recovery: require-nix-shell
    bash scripts/tasks/task-6/redis-recovery.sh
