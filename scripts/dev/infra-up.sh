#!/usr/bin/env bash

set -euo pipefail
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"
source "$SCRIPT_DIR/infra-wait.sh"
source "$SCRIPT_DIR/minio-identity.sh"

prepare_infra
compose up --detach
wait_for_infra
prepare_minio_identity
compose ps
