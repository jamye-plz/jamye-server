#!/usr/bin/env bash

set -euo pipefail

if [[ -z "${IN_NIX_SHELL:-}" ]]; then
  printf '%s\n' 'error: enter with `nix develop path:.` before running task-3b' >&2
  exit 2
fi

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$repo_root"

if [[ ! -d contracts ]]; then
  printf '%s\n' 'error: contracts/ is absent; run the task-3b generate card first' >&2
  exit 2
fi

temporary_root="$(mktemp -d "${TMPDIR:-/tmp}/jamye-contract-check.XXXXXX")"
trap 'rm -rf -- "$temporary_root"' EXIT
generated="$temporary_root/generated"

cargo test --locked --test contract
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

if find contracts "$generated" -type l -print -quit | grep -q .; then
  printf '%s\n' 'error: contract trees must not contain symlinks' >&2
  exit 1
fi

(
  cd contracts
  find . -type f -print | LC_ALL=C sort
) > "$temporary_root/committed-files"
(
  cd "$generated"
  find . -type f -print | LC_ALL=C sort
) > "$temporary_root/generated-files"

if ! cmp -s "$temporary_root/committed-files" "$temporary_root/generated-files"; then
  printf '%s\n' 'error: committed and generated contract allowlists differ' >&2
  diff -u "$temporary_root/committed-files" "$temporary_root/generated-files" >&2 || true
  exit 1
fi

while IFS= read -r relative; do
  relative="${relative#./}"
  if ! cmp -s "contracts/$relative" "$generated/$relative"; then
    printf 'error: contract artifact drift: %s\n' "$relative" >&2
    exit 1
  fi
done < "$temporary_root/committed-files"

printf '%s\n' 'contract artifact allowlist, bytes, provenance, and checksum match'
