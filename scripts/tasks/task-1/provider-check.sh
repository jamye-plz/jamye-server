#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

require_nix_shell
require_compose_provider
require_nix_command rustc
require_nix_command cargo

expected_rust="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' "$REPO_ROOT/rust-toolchain.toml")"
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
