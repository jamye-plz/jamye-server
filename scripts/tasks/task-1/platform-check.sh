#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

require_nix_shell
cd "$REPO_ROOT"

cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- --deny warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
cargo test --locked --test architecture
