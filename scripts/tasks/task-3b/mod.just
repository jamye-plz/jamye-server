set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter with `nix develop path:.` before running task-3b' >&2; exit 2; fi

# Generate the committed C0 snapshot from the explicit pre-publication provenance input.
generate: require-nix-shell
    cargo run --locked --bin generate_contracts -- generate --output contracts --provenance src/contract_generation/provenance.json

# Run task-3b's generator and contract integration tests.
test: require-nix-shell
    cargo test --locked --test contract

# Regenerate in a disposable tree and prove the committed snapshot has no drift.
check: require-nix-shell
    bash scripts/tasks/task-3b/contract-check.sh
