set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter with `nix develop path:.` before running task-5' >&2; exit 2; fi

# Prove the selected D12=A production auth target fails only because task-5 is absent.
red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -e migrations/0002_auth_sessions.sql ]]; then
      printf '%s\n' 'error: RED must run before the task-5 auth migration and implementation exist' >&2
      exit 2
    fi
    cargo test --locked --test auth production_auth_surface_is_absent_before_task_5 -- --exact --nocapture

# Regenerate and verify Cargo/Nix locks after the bounded production-auth dependency delta.
locks: require-nix-shell
    just task-1 locks-create
    just task-1 locks-verify

# Reuse the repository-wide advisory, ban, license, and source policy.
dependency-check: require-nix-shell
    just task-1 dependency-check
