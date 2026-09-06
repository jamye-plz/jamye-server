set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: an active pinned development shell is required before running task-3b' >&2; exit 2; fi

# Generate generic C0 from the explicit pre-publication provenance input; legacy generate is not a prerequisite for check.
generate: require-nix-shell
    cargo run --locked --bin generate_contracts -- generate --output contracts --provenance src/contract_generation/provenance.json

# Run task-3b's generator and contract integration tests.
test: require-nix-shell
    cargo test --locked --test contract

# Run C0 coverage plus the exact committed release-candidate assertion, then verify both profiles independently.
check: require-nix-shell
    bash scripts/tasks/task-3b/contract-check.sh
