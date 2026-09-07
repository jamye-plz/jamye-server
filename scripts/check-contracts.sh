#!/usr/bin/env bash

set -euo pipefail

if [[ -z "${IN_NIX_SHELL:-}" ]]; then
  printf '%s\n' 'error: an active repository Nix devShell is required' >&2
  exit 2
fi

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [[ ! -d contracts ]]; then
  printf '%s\n' 'error: committed contracts/ is absent' >&2
  exit 2
fi

temporary_root="$(mktemp -d "${TMPDIR:-/tmp}/jamye-contract-check.XXXXXX")"
trap 'rm -rf -- "$temporary_root"' EXIT
generated="$temporary_root/generated"
release_candidate_test='contract_generation::allowed_provenance_variants_are_deterministic_and_emit_release_candidate_manifest'

cargo test --locked --test contract
release_candidate_inventory="$(
  cargo test --locked --test production_composition "$release_candidate_test" -- --exact --list
)"
printf '%s\n' "$release_candidate_inventory"
release_candidate_match_count=0
while IFS= read -r inventory_line; do
  if [[ "$inventory_line" == "${release_candidate_test}: test" ]]; then
    ((release_candidate_match_count += 1))
  fi
done <<< "$release_candidate_inventory"
if [[ "$release_candidate_match_count" -ne 1 ]]; then
  printf '%s\n' 'error: required release-candidate contract assertion inventory is invalid' >&2
  exit 1
fi
cargo test --locked --test production_composition \
  "$release_candidate_test" \
  -- --exact
cargo run --locked --bin generate_contracts -- \
  generate \
  --output "$generated" \
  --provenance src/contract_generation/provenance.json
cargo run --locked --bin generate_contracts -- \
  verify \
  --input contracts \
  --provenance src/contract_generation/provenance.json
cargo run --locked --bin generate_contracts -- \
  verify \
  --input "$generated" \
  --provenance src/contract_generation/provenance.json

printf '%s\n' 'contract artifact allowlist, bytes, provenance, and checksum match'
