#!/usr/bin/env bash

set -euo pipefail
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"
source "$SCRIPT_DIR/infra-wait.sh"
source "$SCRIPT_DIR/minio-identity.sh"
source "$SCRIPT_DIR/minio-process.sh"

prepare_infra
remove_legacy_minio_containers
status=0
minio_state || status=$?
case "$status" in
  0|1) ;;
  *) exit "$status" ;;
esac
compose up --detach
start_minio
wait_for_infra
status=0
minio_running || status=$?
if [[ "$status" -ne 0 ]]; then
  if [[ "$status" -eq 1 ]]; then
    printf '%s\n' 'error: local MinIO exited before ownership verification; see .local/infra/minio/minio.log' >&2
  fi
  exit "$status"
fi
prepare_minio_identity
compose ps
