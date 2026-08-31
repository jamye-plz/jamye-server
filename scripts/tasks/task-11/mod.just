set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter with `nix develop path:.` before running task-11' >&2; exit 2; fi

# Sprint-1 absence evidence is frozen once Task-11 surfaces exist.
red: require-nix-shell
    @printf '%s\n' 'error: absent-surface RED is unavailable after the Task-11 scaffold exists; use transition-red' >&2; exit 2

# Record the static exact-0007, forward-only migration RED without PostgreSQL.
migration-red: require-nix-shell
    #!/usr/bin/env bash
    set -uo pipefail
    if [[ -e migrations/0008_account_deletion.sql ]]; then
      printf '%s\n' 'error: migration-absent RED is unavailable after 0008 exists; use the owning Sprint-2 executable migration card' >&2
      exit 2
    fi
    CARGO_NET_OFFLINE=true cargo test --locked --test account_deletion migration::account_deletion_migration_is_exact_0007_forward_only_with_recovery_metadata -- --exact --nocapture

# Reserved for Sprint 2 after 0008 implementation; requires disposable PostgreSQL.
migration-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    CARGO_NET_OFFLINE=true cargo test --locked --test account_deletion migration:: -- --nocapture

# Sprint-2 executable PostgreSQL D5/D10/atomicity RED. The scaffold must compile;
# valid RED is observed account-transition behavior, never a missing symbol or source scan.
transition-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    CARGO_NET_OFFLINE=true cargo test --locked --test account_deletion transition:: -- --nocapture

# The same frozen transition cases become GREEN only after their RED output is recorded.
transition-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    CARGO_NET_OFFLINE=true cargo test --locked --test account_deletion transition:: -- --nocapture

# Sprint-3 executable cleanup lease/object-storage RED. This selector is intentionally
# identical to cleanup-green: behavior, rather than missing symbols, changes between gates.
cleanup-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    CARGO_NET_OFFLINE=true cargo test --locked --test account_deletion cleanup:: -- --nocapture

cleanup-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    CARGO_NET_OFFLINE=true cargo test --locked --test account_deletion cleanup:: -- --nocapture

# Sprint-4 executable U3 HTTP contract RED. RED and GREEN intentionally keep
# the exact same selector; only the production handler behavior may change.
http-contract-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    CARGO_NET_OFFLINE=true cargo test --locked --test account_deletion http:: -- --nocapture

http-contract-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    CARGO_NET_OFFLINE=true cargo test --locked --test account_deletion http:: -- --nocapture

# Sprint-4 executable deletion/push interleaving RED. The tests enter deletion
# through DELETE /api/v1/me and use the real worker plus PostgreSQL adapter.
push-barriers-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    CARGO_NET_OFFLINE=true cargo test --locked --test account_deletion push_barriers:: -- --nocapture

push-barriers-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    CARGO_NET_OFFLINE=true cargo test --locked --test account_deletion push_barriers:: -- --nocapture

# Reserved for Sprints 2-4 after the static account-deletion Router and cleanup runner exist.
integration-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    CARGO_NET_OFFLINE=true cargo test --locked --test account_deletion -- --nocapture

# Reserved for the post-Sprint-4 aggregate verification gate.
aggregate: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    CARGO_NET_OFFLINE=true cargo test --locked --test account_deletion -- --nocapture
    CARGO_NET_OFFLINE=true cargo test --locked --test architecture --all-features

# Apply rustfmt across every Cargo workspace target. This mutates Rust source only.
format: require-nix-shell
    cargo fmt --all

# Reserved quality gates; no passing result is claimed by this RED baseline.
format-check: require-nix-shell
    cargo fmt --all -- --check

clippy: require-nix-shell
    CARGO_NET_OFFLINE=true cargo clippy --locked --all-targets --all-features -- --deny warnings

coverage: require-nix-shell
    @printf '%s\n' 'error: coverage was moved by explicit user decision to the post-Task-12/Task-13 final-binary and declared-tooling gate' >&2; exit 2
