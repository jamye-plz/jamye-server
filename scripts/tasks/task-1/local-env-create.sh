#!/usr/bin/env bash

set -euo pipefail

TASK_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$TASK_DIR/../../.." && pwd)"
TARGET="$REPO_ROOT/.env.local"

if [[ -z "${IN_NIX_SHELL:-}" ]]; then
  printf '%s\n' 'error: this card must run inside `nix develop path:.`' >&2
  exit 2
fi
if [[ -e "$TARGET" ]]; then
  printf '%s\n' 'error: .env.local already exists; refusing to overwrite it' >&2
  exit 2
fi

random_hex() {
  od -An -N24 -tx1 /dev/urandom | tr -d ' \n'
}

postgres_password="$(random_hex)"
redis_password="$(random_hex)"
minio_admin_password="$(random_hex)"
minio_app_password="$(random_hex)"

umask 077
temporary="$(mktemp "$REPO_ROOT/.env.local.tmp.XXXXXX")"
trap 'rm -f -- "$temporary"' EXIT

{
  printf '%s\n' '# Generated disposable local-test values. Never use in production.'
  printf '%s\n' 'JAMYE_ENVIRONMENT=test'
  printf '%s\n' 'JAMYE_LISTEN_ADDR=127.0.0.1:8080'
  printf '%s\n' 'JAMYE_SHUTDOWN_GRACE_SECONDS=10'
  printf '%s\n' 'JAMYE_READINESS_TIMEOUT_MS=750'
  printf 'DATABASE_URL=postgresql://jamye_test:%s@127.0.0.1:5432/jamye_test\n' "$postgres_password"
  printf 'REDIS_URL=redis://:%s@127.0.0.1:6379/0\n' "$redis_password"
  printf '%s\n' 'JAMYE_MINIO_HEALTH_URL=http://127.0.0.1:9000/minio/health/live'
  printf 'JAMYE_TEST_POSTGRES_PASSWORD=%s\n' "$postgres_password"
  printf 'JAMYE_TEST_REDIS_PASSWORD=%s\n' "$redis_password"
  printf '%s\n' 'JAMYE_TEST_MINIO_ADMIN_USER=jamye_test_admin'
  printf 'JAMYE_TEST_MINIO_ADMIN_PASSWORD=%s\n' "$minio_admin_password"
  printf '%s\n' 'JAMYE_TEST_MINIO_APP_USER=jamye_test_app'
  printf 'JAMYE_TEST_MINIO_APP_PASSWORD=%s\n' "$minio_app_password"
} > "$temporary"

chmod 600 "$temporary"
mv -- "$temporary" "$TARGET"
trap - EXIT

printf '%s\n' 'created .env.local with mode 0600 and disposable local-test values'
printf '%s\n' 'the MinIO app credential is identity-only in M0; no bucket or policy is created'
