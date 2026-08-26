#!/usr/bin/env bash

set -euo pipefail

TASK_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "$TASK_DIR/../task-1/_common.sh"

prepare_infra

redis_is_healthy() {
  compose exec -T redis sh -euc \
    'redis-cli --no-auth-warning -a "$JAMYE_TEST_REDIS_PASSWORD" ping | grep -q PONG' \
    >/dev/null 2>&1
}

if ! redis_is_healthy; then
  printf '%s\n' 'error: guarded local Redis is not healthy; run `just task-1 infra-up` first' >&2
  exit 2
fi

coordination_dir="$(mktemp -d "${TMPDIR:-/tmp}/jamye-task-6-redis.XXXXXX")"
test_pid=''
redis_stopped=0

wait_for_redis() {
  local attempt
  for attempt in $(seq 1 30); do
    if redis_is_healthy; then
      return 0
    fi
    sleep 2
  done
  printf '%s\n' 'error: Redis did not become healthy within 60 seconds' >&2
  compose ps >&2 || true
  return 1
}

wait_for_marker() {
  local marker="$1"
  local attempt
  for attempt in $(seq 1 600); do
    if [[ -f "$coordination_dir/$marker" ]]; then
      return 0
    fi
    if [[ -n "$test_pid" ]] && ! kill -0 "$test_pid" 2>/dev/null; then
      wait "$test_pid"
      return 1
    fi
    sleep 0.1
  done
  printf 'error: timed out waiting for Rust recovery marker %s\n' "$marker" >&2
  return 1
}

cleanup() {
  local status=$?
  trap - EXIT

  if [[ -n "$test_pid" ]] && kill -0 "$test_pid" 2>/dev/null; then
    kill "$test_pid" 2>/dev/null || true
    wait "$test_pid" 2>/dev/null || true
  fi
  if [[ "$redis_stopped" -eq 1 ]]; then
    printf '%s\n' 'restoring guarded local Redis after interrupted recovery card' >&2
    compose start redis >/dev/null || true
    wait_for_redis || true
  fi

  rm -f -- \
    "$coordination_dir/ready-to-stop" \
    "$coordination_dir/redis-stopped" \
    "$coordination_dir/ready-to-start" \
    "$coordination_dir/redis-started"
  rmdir -- "$coordination_dir" 2>/dev/null || true
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT TERM HUP

export JAMYE_TASK6_RECOVERY_COORD_DIR="$coordination_dir"
cargo test --locked --test groups \
  redis_recovery::redis_stop_restart_preserves_invite_authority_and_recovers_the_same_limiter -- \
  --ignored --exact --nocapture --test-threads=1 &
test_pid=$!

wait_for_marker ready-to-stop
printf '%s\n' 'stopping only the guarded local Redis container; PostgreSQL and volumes are preserved'
redis_stopped=1
compose stop redis
if redis_is_healthy; then
  printf '%s\n' 'error: Redis still accepts connections after compose stop' >&2
  exit 1
fi
: >"$coordination_dir/redis-stopped"

wait_for_marker ready-to-start
printf '%s\n' 'starting the same guarded local Redis container'
compose start redis
wait_for_redis
redis_stopped=0
: >"$coordination_dir/redis-started"

wait "$test_pid"
test_pid=''
printf '%s\n' 'same Redis limiter recovered; PostgreSQL invite and membership authority was preserved'
