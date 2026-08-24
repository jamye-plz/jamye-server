#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

wait_for_infra() {
  local attempt
  for attempt in $(seq 1 30); do
    if compose exec -T postgres pg_isready -U jamye_test -d jamye_test >/dev/null 2>&1 \
      && compose exec -T redis sh -euc 'redis-cli --no-auth-warning -a "$JAMYE_TEST_REDIS_PASSWORD" ping | grep -q PONG' >/dev/null 2>&1 \
      && curl --fail --silent --show-error http://127.0.0.1:9000/minio/health/live >/dev/null 2>&1; then
      printf '%s\n' 'postgres, redis, and minio are healthy'
      return 0
    fi
    sleep 2
  done

  printf '%s\n' 'error: local infrastructure did not become healthy within 60 seconds' >&2
  compose ps >&2 || true
  return 1
}
