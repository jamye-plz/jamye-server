#!/usr/bin/env bash

set -euo pipefail
TASK_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "$TASK_DIR/_common.sh"
source "$TASK_DIR/_infra-wait.sh"
source "$TASK_DIR/_minio-identity.sh"

prepare_infra
compose up --detach
wait_for_infra
prepare_minio_identity
compose ps
