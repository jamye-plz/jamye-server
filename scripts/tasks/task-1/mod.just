set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter with `nix develop path:.` before running task-1' >&2; exit 2; fi

# Verify the pinned Rust, Podman, and Compose provider without starting services.
provider-check: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    require_compose_provider
    require_nix_command rustc
    require_nix_command cargo

    expected_rust="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' rust-toolchain.toml)"
    actual_rust="$(rustc --version | awk '{print $2}')"
    if [[ -z "$expected_rust" || "$actual_rust" != "$expected_rust" ]]; then
      printf 'error: active rustc %s does not match rust-toolchain.toml %s\n' "$actual_rust" "$expected_rust" >&2
      exit 1
    fi

    printf 'rustc=%s\n' "$(command -v rustc)"
    printf 'cargo=%s\n' "$(command -v cargo)"
    printf 'podman=%s\n' "$(command -v podman)"
    printf 'compose_provider=%s\n' "$PODMAN_COMPOSE_PROVIDER"
    rustc --version
    cargo --version
    podman --version
    "$PODMAN_COMPOSE_PROVIDER" --version
    podman compose version

# Create both dependency lockfiles and print their checksums.
locks-create: require-nix-shell
    cargo generate-lockfile
    nix flake lock path:.
    sha256sum Cargo.lock flake.lock

# Prove locked Cargo and Nix evaluation leave both lockfiles unchanged.
locks-verify: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail

    for lock in Cargo.lock flake.lock; do
      if [[ ! -f "$lock" ]]; then
        printf 'error: missing %s; run `just task-1 locks-create` first\n' "$lock" >&2
        exit 2
      fi
    done

    before="$(sha256sum Cargo.lock flake.lock)"
    cargo metadata --locked --all-features --format-version 1 >/dev/null
    nix flake metadata --no-write-lock-file path:. >/dev/null
    nix flake show --all-systems --no-write-lock-file path:. >/dev/null
    after="$(sha256sum Cargo.lock flake.lock)"

    if [[ "$before" != "$after" ]]; then
      printf '%s\n' 'error: locked resolution changed Cargo.lock or flake.lock' >&2
      printf '%s\n' "$before" >&2
      printf '%s\n' "$after" >&2
      exit 1
    fi

    printf '%s\n' "$after"
    printf '%s\n' 'locked Cargo and Nix evaluation left both lockfiles unchanged'

# Check Cargo advisories, bans, licenses, and sources.
dependency-check: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    require_nix_command cargo-deny
    cargo deny check

# Run the clean/detect/clean secret scanner regression with guarded cleanup.
secret-scan: require-nix-shell
    bash scripts/tasks/task-1/secret-scan.sh

# Create the mode-0600 disposable local environment without overwriting it.
local-env-create: require-nix-shell
    bash scripts/tasks/task-1/local-env-create.sh

# Run formatting, warning-as-error lint, all test modes, and architecture checks.
platform-check: require-nix-shell
    cargo fmt --all -- --check
    cargo clippy --locked --all-targets --all-features -- --deny warnings
    cargo test --locked --all-targets
    cargo test --locked --all-targets --all-features
    cargo test --locked --test architecture

# Start PostgreSQL, Redis, and MinIO, wait for health, and prepare MinIO identity.
infra-up: require-nix-shell
    bash scripts/tasks/task-1/infra-up.sh

# Show the guarded local Compose project status.
infra-status: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    prepare_infra
    compose ps

# Stop local containers and network while preserving named-volume bytes.
infra-down: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    prepare_infra
    compose down --remove-orphans
    printf '%s\n' 'local containers and project network stopped; named-volume bytes were preserved'

# Delete only the three guarded disposable volumes after explicit confirmation.
infra-reset: require-nix-shell
    bash scripts/tasks/task-1/infra-reset.sh

# Run the API with validated values from .env.local.
api-run: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    exec cargo run --locked --bin api

# Run the worker with validated values from .env.local.
worker-run: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    exec cargo run --locked --bin worker

# Validate live and ready responses from the running local API.
health-check: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/tasks/task-1/_common.sh
    load_local_env
    require_nix_command curl
    require_nix_command jq

    base_url="http://$JAMYE_LISTEN_ADDR"
    live_response="$(curl --fail --silent --show-error "$base_url/health/live")"
    ready_response="$(curl --fail --silent --show-error "$base_url/health/ready")"

    jq -e '.status == "live"' <<<"$live_response" >/dev/null
    jq -e '
      .status == "ready"
      and .checks.postgres.status == "ready"
      and .checks.redis.status == "ready"
      and .checks.minio.status == "ready"
    ' <<<"$ready_response" >/dev/null

    jq . <<<"$live_response"
    jq . <<<"$ready_response"

# Evaluate and check the current system without rewriting flake.lock.
flake-local: require-nix-shell
    nix flake show --all-systems --no-write-lock-file path:.
    nix flake check --no-write-lock-file path:.

# Build the production binaries through an explicit x86_64-linux builder.
flake-linux: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    printf '%s\n' 'This card requires an x86_64-linux builder. Failure to find one is an M0 blocker, not a skip.'
    nix build --no-link --no-write-lock-file \
      path:.#packages.x86_64-linux.api \
      path:.#packages.x86_64-linux.worker
