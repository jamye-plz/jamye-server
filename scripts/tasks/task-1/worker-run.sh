#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

require_nix_shell
load_local_env
cd "$REPO_ROOT"
exec cargo run --locked --bin worker
