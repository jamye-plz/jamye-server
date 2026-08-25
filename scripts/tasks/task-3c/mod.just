set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter with `nix develop path:.` before running task-3c' >&2; exit 2; fi

# Prove the guarded dev identity/seed surface is absent before implementation.
red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -e src/dev_fixtures/mod.rs ]]; then
      printf '%s\n' 'error: RED must run before the task-3c dev fixture implementation exists' >&2
      exit 2
    fi
    cargo test --locked --test dev_fixtures dev_fixture_surface_is_feature_and_environment_guarded -- --exact --nocapture

# Regenerate and verify the Cargo/Nix locks after the approved optional JWT dependency.
locks: require-nix-shell
    just task-1 locks-create
    just task-1 locks-verify

# Reuse the repository-wide advisory, ban, license, and source policy.
dependency-check: require-nix-shell
    just task-1 dependency-check

# Prove default-build absence, guarded JWT behavior, atomic seed persistence, and architecture.
green: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env

    JAMYE_ENABLE_DEV_FIXTURES=true \
      cargo test --locked --no-default-features --test dev_fixtures \
      dev_fixture_surface_is_feature_and_environment_guarded -- --exact --nocapture
    JAMYE_ENABLE_DEV_FIXTURES=true \
      cargo test --locked --lib --features dev-fixtures
    JAMYE_ENABLE_DEV_FIXTURES=true \
      cargo test --locked --test dev_fixtures --features dev-fixtures -- --nocapture
    cargo test --locked --test architecture --all-features
