#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

require_nix_shell
require_nix_command cargo-deny
cd "$REPO_ROOT"
cargo deny check
