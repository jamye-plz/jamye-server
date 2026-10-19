set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list

# Show the complete public workflow.
default:
    @just --list

# Enter the pinned development shell without starting services.
shell:
    nix develop .

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter the repository Nix devShell before running this command' >&2; exit 2; fi

# Format Rust sources.
format: require-nix-shell
    cargo fmt --all

# Check Rust formatting without changing files.
format-check: require-nix-shell
    cargo fmt --all -- --check

# Run Clippy for every target and feature with warnings denied.
clippy: require-nix-shell
    CARGO_NET_OFFLINE=true cargo clippy --locked --all-targets --all-features -- --deny warnings

# Run the complete default and all-feature test suites against local services.
test: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/dev/common.sh
    load_local_env
    CARGO_NET_OFFLINE=true cargo test --locked --all-targets
    JAMYE_ENABLE_DEV_FIXTURES=true CARGO_NET_OFFLINE=true \
      cargo test --locked --all-targets --all-features

# Regenerate the committed release-candidate API and realtime contracts.
contract-generate: require-nix-shell
    cargo run --locked --bin generate_contracts -- generate-release-candidate --output contracts --provenance tests/production_composition/fixtures/contract_generation/dirty.json

# Verify that committed contracts are deterministic and current.
contract-check: require-nix-shell
    bash scripts/check-contracts.sh

# Primary implementation-complete gate.
check: format-check clippy test contract-check
    @printf '%s\n' 'format, Clippy, tests, and contract drift checks passed'

# Produce an informational all-feature coverage report; this is not a completion gate.
coverage: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/dev/common.sh
    load_local_env
    JAMYE_ENABLE_DEV_FIXTURES=true CARGO_NET_OFFLINE=true \
      cargo llvm-cov --locked --offline --workspace --all-targets --all-features -- \
      --test-threads=1

# Check dependency advisories, bans, licenses, and sources.
dependency-check: require-nix-shell
    cargo deny check

# Scan tracked and unignored working-tree content for secrets.
secret-check: require-nix-shell
    bash scripts/dev/check-secrets.sh

# Verify locked Cargo and Nix evaluation without rewriting either lockfile.
lock-check: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    for lock in Cargo.lock flake.lock; do
      [[ -f "$lock" ]] || { printf 'error: missing %s\n' "$lock" >&2; exit 2; }
    done
    before="$(sha256sum Cargo.lock flake.lock)"
    cargo metadata --locked --all-features --format-version 1 >/dev/null
    nix flake metadata --no-write-lock-file . >/dev/null
    nix flake show --all-systems --no-write-lock-file . >/dev/null
    after="$(sha256sum Cargo.lock flake.lock)"
    [[ "$before" == "$after" ]] || { printf '%s\n' 'error: Cargo.lock or flake.lock changed' >&2; exit 1; }
    printf '%s\n' "$after"

# Evaluate and check the flake on the current host without rewriting flake.lock.
flake-check: require-nix-shell
    nix flake show --all-systems --no-write-lock-file .
    nix flake check --no-write-lock-file .

# Verify the pinned Rust, Cargo, Podman, and Compose tools.
tools-check: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/dev/common.sh
    require_compose_provider
    require_nix_command rustc
    require_nix_command cargo
    expected="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' rust-toolchain.toml)"
    actual="$(rustc --version | awk '{print $2}')"
    [[ -n "$expected" && "$actual" == "$expected" ]] || { printf 'error: rustc %s does not match %s\n' "$actual" "$expected" >&2; exit 1; }
    rustc --version
    cargo --version
    podman --version
    "$PODMAN_COMPOSE_PROVIDER" --version

# Create a mode-0600 disposable local environment without overwriting one.
env-create: require-nix-shell
    bash scripts/dev/create-local-env.sh

# Start PostgreSQL, Redis, and MinIO and wait for health.
infra-up: require-nix-shell
    bash scripts/dev/infra-up.sh

# Show the guarded local service status.
infra-status: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/dev/common.sh
    source scripts/dev/minio-process.sh
    prepare_infra
    compose ps
    legacy_containers="$(legacy_minio_containers)"
    if [[ -n "$legacy_containers" ]]; then
      printf '%s\n' 'legacy Compose MinIO container present; run `just infra-up` or `just infra-down` to migrate'
    else
      status_minio
    fi

# Stop local services while preserving their data.
infra-down: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/dev/common.sh
    source scripts/dev/minio-process.sh
    prepare_infra
    legacy_status=0
    remove_legacy_minio_containers || legacy_status=$?
    minio_status=0
    # Capture safety exits in a subshell so Compose teardown still runs.
    (stop_minio) || minio_status=$?
    compose_status=0
    compose down --remove-orphans || compose_status=$?
    if [[ "$legacy_status" -ne 0 ]]; then exit "$legacy_status"; fi
    if [[ "$minio_status" -ne 0 ]]; then exit "$minio_status"; fi
    if [[ "$compose_status" -ne 0 ]]; then exit "$compose_status"; fi
    printf '%s\n' 'local services stopped; data was preserved'

# Delete guarded disposable infrastructure data after explicit confirmation.
infra-reset: require-nix-shell
    bash scripts/dev/infra-reset.sh

# Attach the least-privilege policy to the disposable MinIO app identity.
minio-policy: require-nix-shell
    bash scripts/dev/apply-minio-policy.sh

# Run the API with values loaded from .env.local.
run-api: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/dev/common.sh
    load_local_env
    exec cargo run --locked --bin api

# Run the worker with values loaded from .env.local.
run-worker: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/dev/common.sh
    load_local_env
    exec cargo run --locked --bin worker

# Check live and ready responses from a running local API.
health-check: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/dev/common.sh
    load_local_env
    require_nix_command curl
    require_nix_command jq
    base_url="http://$JAMYE_LISTEN_ADDR"
    live="$(curl --fail --silent --show-error "$base_url/health/live")"
    ready="$(curl --fail --silent --show-error "$base_url/health/ready")"
    jq -e '.status == "live"' <<<"$live" >/dev/null
    jq -e '.status == "ready" and .checks.postgres.status == "ready"' <<<"$ready" >/dev/null
    jq . <<<"$live"
    jq . <<<"$ready"

# Run the four explicit service stop/start recovery tests.
test-recovery: require-nix-shell
    bash scripts/recovery/postgres.sh
    bash scripts/recovery/redis.sh realtime
    bash scripts/recovery/redis.sh rate-limit
    bash scripts/recovery/redis.sh groups
