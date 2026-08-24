#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

require_nix_shell
cd "$REPO_ROOT"

printf '%s\n' 'This card requires an x86_64-linux builder. Failure to find one is an M0 blocker, not a skip.'
nix build --no-link --no-write-lock-file \
  path:.#packages.x86_64-linux.api \
  path:.#packages.x86_64-linux.worker
