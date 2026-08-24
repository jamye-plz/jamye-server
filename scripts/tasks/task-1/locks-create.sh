#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

require_nix_shell
cd "$REPO_ROOT"

cargo generate-lockfile
nix flake lock path:.
sha256sum Cargo.lock flake.lock
