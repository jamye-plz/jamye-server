#!/usr/bin/env bash

MINIO_DIR="$REPO_ROOT/.local/infra/minio"
MINIO_DATA_DIR="$MINIO_DIR/data"
MINIO_PID_FILE="$MINIO_DIR/minio.pid"
MINIO_LOG_FILE="$MINIO_DIR/minio.log"

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
    *) printf 'error: PID %s is not the local MinIO process\n' "$pid" >&2; return 2 ;;
  esac
  if [[ "$process" != *"$binary"*"$MINIO_DATA_DIR"* ]]; then
    printf 'error: PID %s does not use the local MinIO data directory\n' "$pid" >&2
    return 2
  fi
  if minio_port_in_use && ! minio_owns_port "$pid"; then
    printf '%s\n' 'error: port 9000 is not owned by the local MinIO PID' >&2
    return 2
  fi
  return 0
}

minio_port_in_use() {
  (echo >/dev/tcp/127.0.0.1/9000) >/dev/null 2>&1
}

minio_owns_port() {
  local pid="$1"
  case "$(uname -s)" in
    Darwin)
      lsof -nP -a -p "$pid" -iTCP:9000 -sTCP:LISTEN -Fn 2>/dev/null \
        | grep -Fxq 'n127.0.0.1:9000'
      ;;
    Linux)
      ss -H -lntp 2>/dev/null \
        | awk -v owner="pid=$pid," '$4 == "127.0.0.1:9000" && index($0, owner) {found=1} END {exit !found}'
      ;;
    *) return 1 ;;
  esac
}

start_minio() {
  local status package
  if minio_running; then
    printf '%s\n' 'local MinIO is already running'
    return 0
  else
    status=$?
    [[ "$status" -eq 1 ]] || return "$status"
  fi
  if minio_port_in_use; then
    printf '%s\n' 'error: port 9000 is occupied by an unmanaged process' >&2
    return 2
  fi

  require_command nix
  require_value JAMYE_TEST_MINIO_ADMIN_USER
  require_value JAMYE_TEST_MINIO_ADMIN_PASSWORD
  if [[ -L "$MINIO_DIR" || "$(realpath -m -- "$MINIO_DIR")" != "$MINIO_DIR" ]]; then
    printf '%s\n' 'error: unsafe MinIO state directory' >&2
    return 2
  fi
  mkdir -p -- "$MINIO_DATA_DIR"
  package="$(cd -- "$REPO_ROOT" && nix build --no-link --print-out-paths --no-write-lock-file .#minio)"
  MINIO_ROOT_USER="$JAMYE_TEST_MINIO_ADMIN_USER" \
    MINIO_ROOT_PASSWORD="$JAMYE_TEST_MINIO_ADMIN_PASSWORD" \
    nohup "$package/bin/minio" server "$MINIO_DATA_DIR" \
      --address 127.0.0.1:9000 --console-address 127.0.0.1:9001 \
      >"$MINIO_LOG_FILE" 2>&1 </dev/null &
  printf '%s\n' "$!" > "$MINIO_PID_FILE"
}

status_minio() {
  local status
  if minio_running; then
    printf 'local MinIO running (PID %s)\n' "$(cat "$MINIO_PID_FILE")"
  else
    status=$?
    [[ "$status" -eq 1 ]] || return "$status"
    if minio_port_in_use; then
      printf '%s\n' 'error: port 9000 is occupied by an unmanaged process' >&2
      return 2
    fi
    printf '%s\n' 'local MinIO is stopped'
  fi
}

stop_minio() {
  local status pid attempt
  if minio_running; then
    pid="$(cat "$MINIO_PID_FILE")"
    kill -TERM "$pid"
    for attempt in {1..10}; do
      sleep 1
      if minio_running; then
        continue
      else
        status=$?
        [[ "$status" -eq 1 ]] || return "$status"
        rm -f -- "$MINIO_PID_FILE"
        printf '%s\n' 'local MinIO stopped'
        return 0
      fi
    done
    printf 'error: MinIO PID %s did not stop within 10 seconds\n' "$pid" >&2
    return 1
  else
    status=$?
    [[ "$status" -eq 1 ]] || return "$status"
    if minio_port_in_use; then
      printf '%s\n' 'error: port 9000 is occupied by an unmanaged process' >&2
      return 2
    fi
    rm -f -- "$MINIO_PID_FILE"
    printf '%s\n' 'local MinIO is already stopped'
  fi
}
