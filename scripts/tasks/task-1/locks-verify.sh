#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

require_nix_shell
cd "$REPO_ROOT"

for lock in Cargo.lock flake.lock; do
  if [[ ! -f "$lock" ]]; then
    printf 'error: missing %s; run the task-1 locks-create card first\n' "$lock" >&2
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
