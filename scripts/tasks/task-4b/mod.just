set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter with `nix develop path:.` before running task-4b' >&2; exit 2; fi

[private]
redis-publish-config-inventory:
    #!/usr/bin/env bash
    set -euo pipefail
    expected_names="$(printf '%s\n' \
      'config::realtime::redis_publish_config_binding::defaults_are_exact_15000_2000_1000_ms' \
      'config::realtime::redis_publish_config_binding::key_only_errors_never_echo_raw_values' \
      'config::realtime::redis_publish_config_binding::partial_override_uses_remaining_defaults' \
      'transport::realtime::composition::redis_publish_config_binding::all_three_overrides_materialize_outbox_worker_durations' \
      'transport::realtime::composition::redis_publish_config_binding::arithmetic_overflow_fails_before_side_effects' \
      'transport::realtime::composition::redis_publish_config_binding::equality_and_over_budget_fail_before_side_effects' \
      'transport::realtime::composition::redis_publish_config_binding::existing_realtime_worker_default_regression' \
      'transport::realtime::composition::redis_publish_config_binding::invalid_parse_and_bounds_fail_before_side_effects')"
    actual_names="$(CARGO_NET_OFFLINE=true cargo test --locked --lib -- --list | awk '/redis_publish_config_binding::/ { sub(/: test$/, ""); print }' | LC_ALL=C sort)"
    if [[ "${actual_names}" != "${expected_names}" ]]; then
      printf '%s\n' 'error: redis publish config inventory differs from the frozen lexical oracle' >&2
      exit 2
    fi

# Replays the accepted focused RED contract without mutating its frozen inventory.
redis-publish-config-red: require-nix-shell redis-publish-config-inventory
    CARGO_NET_OFFLINE=true cargo test --locked --lib redis_publish_config_binding -- --nocapture

# Runs the same frozen inventory and selector after the GREEN implementation.
redis-publish-config-green: require-nix-shell redis-publish-config-inventory
    CARGO_NET_OFFLINE=true cargo test --locked --lib redis_publish_config_binding -- --nocapture

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
