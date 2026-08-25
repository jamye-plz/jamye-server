#!/usr/bin/env bash

set -euo pipefail

TASK_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "$TASK_DIR/../task-1/_common.sh"

prepare_infra

if ! compose exec -T postgres pg_isready -U jamye_test -d jamye_test >/dev/null 2>&1; then
  printf '%s\n' 'error: guarded local PostgreSQL is not healthy; run `just task-1 infra-up` first' >&2
  exit 2
fi

JAMYE_ENABLE_DEV_FIXTURES=true \
  cargo test --locked --test messaging --features dev-fixtures 'recovery::' -- \
  --skip 'postgres_stop_restart_keeps_the_same_router_alive_and_recovers' --nocapture

coordination_dir="$(mktemp -d "${TMPDIR:-/tmp}/jamye-task-4a-postgres.XXXXXX")"
test_pid=''
postgres_stopped=0

wait_for_postgres() {
  local attempt
  for attempt in $(seq 1 30); do
    if compose exec -T postgres pg_isready -U jamye_test -d jamye_test >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  printf '%s\n' 'error: PostgreSQL did not become healthy within 60 seconds' >&2
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
  if [[ "$postgres_stopped" -eq 1 ]]; then
    printf '%s\n' 'restoring guarded local PostgreSQL after interrupted recovery card' >&2
    compose start postgres >/dev/null || true
    wait_for_postgres || true
  fi

  rm -f -- \
    "$coordination_dir/ready-to-stop" \
    "$coordination_dir/postgres-stopped" \
    "$coordination_dir/ready-to-start" \
    "$coordination_dir/postgres-started"
  rmdir -- "$coordination_dir" 2>/dev/null || true
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT TERM HUP

export JAMYE_TASK4A_RECOVERY_COORD_DIR="$coordination_dir"
JAMYE_ENABLE_DEV_FIXTURES=true \
  cargo test --locked --test messaging --features dev-fixtures \
  'recovery::postgres_stop_restart_keeps_the_same_router_alive_and_recovers' -- \
  --ignored --exact --nocapture --test-threads=1 &
test_pid=$!

wait_for_marker ready-to-stop
printf '%s\n' 'stopping only the guarded local PostgreSQL container; volumes are preserved'
postgres_stopped=1
compose stop postgres
if compose exec -T postgres pg_isready -U jamye_test -d jamye_test >/dev/null 2>&1; then
  printf '%s\n' 'error: PostgreSQL still accepts connections after compose stop' >&2
  exit 1
fi
: >"$coordination_dir/postgres-stopped"

wait_for_marker ready-to-start
printf '%s\n' 'starting the same guarded local PostgreSQL container'
compose start postgres
wait_for_postgres
postgres_stopped=0
: >"$coordination_dir/postgres-started"

wait "$test_pid"
test_pid=''
printf '%s\n' 'same-router PostgreSQL stop/start recovery passed; named-volume bytes were preserved'
