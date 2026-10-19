#!/usr/bin/env bash

set -euo pipefail
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"
source "$SCRIPT_DIR/infra-wait.sh"
source "$SCRIPT_DIR/minio-identity.sh"
source "$SCRIPT_DIR/minio-process.sh"

prepare_infra
legacy_containers="$("$engine" ps -a -q \
  --filter "label=com.docker.compose.project=$COMPOSE_PROJECT" \
  --filter 'label=com.docker.compose.service=minio')"
if [[ -n "$legacy_containers" ]]; then
  legacy_ids=()
  while IFS= read -r container_id; do
    legacy_ids+=("$container_id")
  done <<< "$legacy_containers"
  "$engine" rm --force "${legacy_ids[@]}"
fi
compose up --detach
start_minio
wait_for_infra
minio_running
prepare_minio_identity
compose ps
