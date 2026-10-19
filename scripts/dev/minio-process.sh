#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/common.sh"

MINIO_DIR="$REPO_ROOT/.local/infra/minio"
MINIO_DATA_DIR="$MINIO_DIR/data"
MINIO_PID_FILE="$MINIO_DIR/minio.pid"
MINIO_LOG_FILE="$MINIO_DIR/minio.log"

require_safe_minio_dir() {
  if [[ -L "$MINIO_DIR" || "$(realpath -m -- "$MINIO_DIR")" != "$MINIO_DIR" ]]; then
    printf '%s\n' 'error: unsafe MinIO state directory' >&2
    exit 2
  fi
}

# Return 0 only for our live process, 1 for no process, and 2 for a
# PID that belongs to another process. Never signal a mismatched PID.
minio_running() {
  local pid process binary state
  [[ -f "$MINIO_PID_FILE" ]] || return 1
  pid="$(cat "$MINIO_PID_FILE")"
  if [[ ! "$pid" =~ ^[1-9][0-9]*$ ]]; then
    printf '%s\n' 'error: invalid MinIO PID file' >&2
    return 2
  fi
  require_command ps
  if ! process="$(ps -p "$pid" -o command= 2>/dev/null)" || [[ -z "$process" ]]; then
    if kill -0 "$pid" 2>/dev/null; then
      printf 'error: cannot verify the identity of PID %s\n' "$pid" >&2
      return 2
    fi
    return 1
  fi
  state="$(ps -p "$pid" -o stat= 2>/dev/null || true)"
  [[ "$state" == *Z* ]] && return 1
  binary="${process%% *}"
  case "$binary" in
    /nix/store/*/bin/minio) ;;
    *) printf 'error: PID %s is not the local MinIO process; if %s is stale, verify and remove it\n' "$pid" "$MINIO_PID_FILE" >&2; return 2 ;;
  esac
  if [[ "$process" != *"$binary"*"$MINIO_DATA_DIR"* ]]; then
    printf 'error: PID %s does not use the local MinIO data directory; if %s is stale, verify and remove it\n' "$pid" "$MINIO_PID_FILE" >&2
    return 2
  fi
  if minio_port_in_use && ! minio_owns_port "$pid"; then
    printf '%s\n' 'error: port 9000 is not owned by the local MinIO PID' >&2
    return 2
  fi
  return 0
}

# A stopped PID is safe only when the port is also free.
minio_state() {
  local status=0 legacy_containers
  minio_running || status=$?
  if [[ "$status" -eq 1 ]] && minio_port_in_use; then
    legacy_containers="$(legacy_minio_containers)" || return $?
    if [[ -n "$legacy_containers" ]]; then
      printf '%s\n' 'error: port 9000 is occupied and a legacy compose MinIO container exists; run `just infra-up` or `just infra-down` to remove it' >&2
    else
      printf '%s\n' 'error: port 9000 is occupied by an unmanaged process' >&2
    fi
    return 2
  fi
  return "$status"
}

minio_port_in_use() {
  (echo >/dev/tcp/127.0.0.1/9000) >/dev/null 2>&1
}

minio_owns_port() {
  local pid="$1"
  case "$(uname -s)" in
    Darwin)
      require_command lsof
      lsof -nP -a -p "$pid" -iTCP:9000 -sTCP:LISTEN -Fn 2>/dev/null \
        | grep -Fxq 'n127.0.0.1:9000'
      ;;
    Linux)
      require_command ss
      ss -H -lntp 2>/dev/null \
        | awk -v owner="pid=$pid," '$4 == "127.0.0.1:9000" && index($0, owner) {found=1} END {exit !found}'
      ;;
    *) return 1 ;;
  esac
}

legacy_minio_containers() {
  "$CONTAINER_ENGINE" ps -a -q \
    --filter "label=com.docker.compose.project=$COMPOSE_PROJECT" \
    --filter 'label=com.docker.compose.service=minio'
}

# podman-compose 1.6.0 does not remove the legacy container with up --remove-orphans.
remove_legacy_minio_container() {
  local legacy_containers container_id
  local -a legacy_ids=()
  legacy_containers="$(legacy_minio_containers)" || return $?
  if [[ -n "$legacy_containers" ]]; then
    while IFS= read -r container_id; do
      legacy_ids+=("$container_id")
    done <<< "$legacy_containers"
    "$CONTAINER_ENGINE" rm --force "${legacy_ids[@]}"
  fi
}

start_minio() {
  local status=0 package path
  minio_state || status=$?
  case "$status" in
    0)
      require_safe_minio_dir
      chmod 700 -- "$MINIO_DIR" "$MINIO_DATA_DIR"
      for path in "$MINIO_LOG_FILE" "$MINIO_PID_FILE"; do
        if [[ -f "$path" ]]; then chmod 600 -- "$path"; fi
      done
      printf '%s\n' 'local MinIO is already running'
      return 0
      ;;
    1) ;;
    *) return "$status" ;;
  esac

  require_command nix
  require_value JAMYE_TEST_MINIO_ADMIN_USER
  require_value JAMYE_TEST_MINIO_ADMIN_PASSWORD
  require_safe_minio_dir
  (umask 077; mkdir -p -- "$MINIO_DATA_DIR")
  chmod 700 -- "$MINIO_DIR" "$MINIO_DATA_DIR"
  package="$(cd -- "$REPO_ROOT" && nix build --no-link --print-out-paths --no-write-lock-file .#minio)"
  (
    umask 077
    : >> "$MINIO_LOG_FILE"
    : > "$MINIO_PID_FILE"
    chmod 600 -- "$MINIO_LOG_FILE" "$MINIO_PID_FILE"
    MINIO_ROOT_USER="$JAMYE_TEST_MINIO_ADMIN_USER" \
      MINIO_ROOT_PASSWORD="$JAMYE_TEST_MINIO_ADMIN_PASSWORD" \
      nohup "$package/bin/minio" server "$MINIO_DATA_DIR" \
        --address 127.0.0.1:9000 --console-address 127.0.0.1:9001 \
        >"$MINIO_LOG_FILE" 2>&1 </dev/null &
    printf '%s\n' "$!" > "$MINIO_PID_FILE"
  )
}

status_minio() {
  local status=0
  minio_state || status=$?
  case "$status" in
    0) printf 'local MinIO running (PID %s)\n' "$(cat "$MINIO_PID_FILE")" ;;
    1) printf '%s\n' 'local MinIO is stopped' ;;
    *) return "$status" ;;
  esac
}

stop_minio() {
  local status=0 pid attempt
  minio_state || status=$?
  case "$status" in
    0) ;;
    1) rm -f -- "$MINIO_PID_FILE"; printf '%s\n' 'local MinIO is already stopped'; return 0 ;;
    *) return "$status" ;;
  esac
  pid="$(cat "$MINIO_PID_FILE")"
  kill -TERM "$pid"
  for attempt in {1..10}; do
    sleep 1
    status=0
    minio_running || status=$?
    case "$status" in
      0) continue ;;
      1) rm -f -- "$MINIO_PID_FILE"; printf '%s\n' 'local MinIO stopped'; return 0 ;;
      *) return "$status" ;;
    esac
  done
  printf 'error: MinIO PID %s did not stop within 10 seconds\n' "$pid" >&2
  return 1
}
