#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

require_nix_shell
cd "$REPO_ROOT"

nix flake show --all-systems --no-write-lock-file path:.
nix flake check --no-write-lock-file path:.
