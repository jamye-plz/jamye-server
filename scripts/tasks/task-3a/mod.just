set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter with `nix develop path:.` before running task-3a' >&2; exit 2; fi

# Prove the schema test fails because 0001 has not been authored yet.
red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -e migrations/0001_core_reliable_messaging.sql ]]; then
      printf '%s\n' 'error: RED must run before the task-3a production migration exists' >&2
      exit 2
    fi
    cargo test --locked --test core_schema migration_source_is_exactly_forward_only_0001 -- --exact --nocapture

# Apply 0001 only to guarded, randomly named databases under local jamye_test.
green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    cargo test --locked --test core_schema -- --nocapture
