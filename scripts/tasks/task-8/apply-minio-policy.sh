#!/usr/bin/env bash

set -euo pipefail

TASK_8_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "$TASK_8_DIR/../task-1/_common.sh"

require_nix_shell
load_local_env
require_nix_command curl
require_nix_command mc
require_value JAMYE_TEST_MINIO_ADMIN_USER
require_value JAMYE_TEST_MINIO_ADMIN_PASSWORD
require_value JAMYE_TEST_MINIO_APP_USER
require_value JAMYE_TEST_MINIO_APP_PASSWORD

if [[ "${JAMYE_ENVIRONMENT:-}" != "test" ]] \
  || [[ "${JAMYE_MINIO_HEALTH_URL:-}" != "http://127.0.0.1:9000/minio/health/live" ]]; then
  printf '%s\n' 'error: task-8 MinIO policy accepts only the task-1 loopback test environment' >&2
  exit 2
fi
if [[ "$JAMYE_TEST_MINIO_ADMIN_USER" == "$JAMYE_TEST_MINIO_APP_USER" ]] \
  || [[ "$JAMYE_TEST_MINIO_ADMIN_PASSWORD" == "$JAMYE_TEST_MINIO_APP_PASSWORD" ]]; then
  printf '%s\n' 'error: MinIO admin and app credentials must remain distinct' >&2
  exit 2
fi

curl --fail --silent --show-error --max-time 3 "$JAMYE_MINIO_HEALTH_URL" >/dev/null

policy_path="$REPO_ROOT/scripts/tasks/task-8/minio-app-policy.json"
if [[ ! -f "$policy_path" ]]; then
  printf 'error: task-8 MinIO policy is missing: %s\n' "$policy_path" >&2
  exit 2
fi

alias_name="jamye_task8_bootstrap"
alias_variable="MC_HOST_${alias_name}"
cleanup_alias() {
  unset "$alias_variable"
}
trap cleanup_alias EXIT

export "$alias_variable=http://${JAMYE_TEST_MINIO_ADMIN_USER}:${JAMYE_TEST_MINIO_ADMIN_PASSWORD}@127.0.0.1:9000"
mc admin info "$alias_name" >/dev/null
if ! mc admin user info "$alias_name" "$JAMYE_TEST_MINIO_APP_USER" >/dev/null 2>&1; then
  printf '%s\n' 'error: disposable MinIO app identity is absent; run the task-1 infra-up card' >&2
  exit 2
fi

mc admin policy create "$alias_name" jamye-task8-media "$policy_path" >/dev/null
mc admin policy attach "$alias_name" jamye-task8-media \
  --user "$JAMYE_TEST_MINIO_APP_USER" >/dev/null

printf '%s\n' 'attached the exact task-8 disposable MinIO app policy'
