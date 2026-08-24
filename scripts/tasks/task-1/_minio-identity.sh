#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

prepare_minio_identity() {
  local alias_name="jamye_test_bootstrap"

  require_nix_command mc
  require_value JAMYE_TEST_MINIO_ADMIN_USER
  require_value JAMYE_TEST_MINIO_ADMIN_PASSWORD
  require_value JAMYE_TEST_MINIO_APP_USER
  require_value JAMYE_TEST_MINIO_APP_PASSWORD

  if [[ "$JAMYE_TEST_MINIO_ADMIN_USER" == "$JAMYE_TEST_MINIO_APP_USER" ]] \
    || [[ "$JAMYE_TEST_MINIO_ADMIN_PASSWORD" == "$JAMYE_TEST_MINIO_APP_PASSWORD" ]]; then
    printf '%s\n' 'error: MinIO admin and app credentials must be distinct' >&2
    return 1
  fi

  export "MC_HOST_${alias_name}=http://${JAMYE_TEST_MINIO_ADMIN_USER}:${JAMYE_TEST_MINIO_ADMIN_PASSWORD}@127.0.0.1:9000"
  mc admin info "$alias_name" >/dev/null
  if ! mc admin user info "$alias_name" "$JAMYE_TEST_MINIO_APP_USER" >/dev/null 2>&1; then
    mc admin user add "$alias_name" "$JAMYE_TEST_MINIO_APP_USER" "$JAMYE_TEST_MINIO_APP_PASSWORD" >/dev/null
  fi
  mc admin user info "$alias_name" "$JAMYE_TEST_MINIO_APP_USER" >/dev/null

  unset "MC_HOST_${alias_name}"
  printf '%s\n' 'MinIO disposable admin and separate unprivileged app identity are ready'
  printf '%s\n' 'M0 created no bucket, app policy, or lifecycle; task-8 materializes D11'
}
