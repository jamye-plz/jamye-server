set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter with `nix develop path:.` before running task-12' >&2; exit 2; fi

# Sprint-2 direct-call RED: runtime call order and transaction outcome only.
uow-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    CARGO_NET_OFFLINE=true cargo test --locked --test production_composition -- --skip 'composition::' --skip 'migration_chain::' --nocapture

# The same selector becomes GREEN only after the exact three compositions exist.
uow-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    CARGO_NET_OFFLINE=true cargo test --locked --test production_composition -- --skip 'composition::' --skip 'migration_chain::' --nocapture

# Reprove the shared Task-4a HTTP/idempotency surface after enriching its internal result.
messaging-regression: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    JAMYE_ENABLE_DEV_FIXTURES=true CARGO_NET_OFFLINE=true \
      cargo test --locked --test messaging --features dev-fixtures -- \
      --skip 'recovery::' --nocapture

# Reprove the non-ignored realtime C1 flow that shares Task-4a message retry persistence.
realtime-regression: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    JAMYE_ENABLE_DEV_FIXTURES=true CARGO_NET_OFFLINE=true \
      cargo test --locked --test realtime --features dev-fixtures -- \
      --skip 'redis_recovery::' --nocapture

# Sprint-3 black-box composition RED. The selector calls the public Axum
# production router; it never treats a source scan or composition trace as proof.
composition-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    CARGO_NET_OFFLINE=true cargo test --locked --test production_composition composition:: -- --nocapture

# The exact same observed-router selector becomes GREEN only after valid RED
# evidence has authorized final static API/worker composition.
composition-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    CARGO_NET_OFFLINE=true cargo test --locked --test production_composition composition:: -- --nocapture

# Open after valid final composition GREEN. The chain test uses only the guarded
# disposable PostgreSQL test database and covers fresh 0001 through 0008.
migration-chain-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    CARGO_NET_OFFLINE=true cargo test --locked --test production_composition migration_chain:: -- --nocapture

# Sprint-4 contract-generation RED. This is the production-composition child
# selector; it observes generated artifacts and never invokes tests/contract.
contract-red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    CARGO_NET_OFFLINE=true cargo test --locked --test production_composition contract_generation:: -- --nocapture

# Reserved for the approved Sprint-4 implementation. Keep the same selector
# so GREEN proves the same generated-artifact contracts as RED.
contract-green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    CARGO_NET_OFFLINE=true cargo test --locked --test production_composition contract_generation:: -- --nocapture

# Focused G5 diagnostic: exercise the complete media target under the default
# parallel test harness five times with the same validated local environment.
media-parallel: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    media_parallel_exit=0
    for run in 1 2 3 4 5; do
      printf 'task_12_g5_media_parallel_run=%s\n' "$run"
      if CARGO_NET_OFFLINE=true cargo test --locked --test media -- --nocapture; then
        continue
      else
        media_parallel_exit=$?
        break
      fi
    done

    exit "$media_parallel_exit"

# Focused G5 diagnostic: exercise the all-features messaging target under the
# default parallel test harness five times with the aggregate environment.
messaging-parallel: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    messaging_parallel_exit=0
    for run in 1 2 3 4 5; do
      printf 'task_12_g5_messaging_parallel_run=%s\n' "$run"
      if JAMYE_ENABLE_DEV_FIXTURES=true CARGO_NET_OFFLINE=true \
        cargo test --locked --test messaging --all-features -- --nocapture; then
        continue
      else
        messaging_parallel_exit=$?
        break
      fi
    done

    exit "$messaging_parallel_exit"

# Focused G5 regression for the selected public realtime event union.
realtime-membership-contract: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    CARGO_NET_OFFLINE=true cargo test --locked \
      --test realtime_membership \
      contract::internal_control_types_never_enter_the_public_realtime_event_union \
      -- --exact --nocapture

# Final G5 cards are thin dispatches to the approved Task-1 stable targets.
format-check: require-nix-shell
    just --justfile scripts/tasks/task-1/mod.just format-check

clippy: require-nix-shell
    just --justfile scripts/tasks/task-1/mod.just clippy

architecture: require-nix-shell
    just --justfile scripts/tasks/task-1/mod.just architecture

aggregate: require-nix-shell
    just --justfile scripts/tasks/task-1/mod.just aggregate
