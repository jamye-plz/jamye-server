#!/usr/bin/env bash

set -euo pipefail

TASK_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$TASK_DIR/../../.." && pwd)"
COMPOSE_FILE="$REPO_ROOT/compose.yaml"
LOCAL_ENV_FILE="$REPO_ROOT/.env.local"
COMPOSE_PROJECT="jamye-server-test"

require_nix_shell() {
  if [[ -z "${IN_NIX_SHELL:-}" ]]; then
    printf '%s\n' 'error: this card must run inside `nix develop path:.`' >&2
    exit 2
  fi
}

require_command() {
  local name="$1"
  if ! command -v "$name" >/dev/null 2>&1; then
    printf 'error: required command is unavailable: %s\n' "$name" >&2
    exit 2
  fi
}

require_nix_command() {
  local name="$1"
  local resolved
  require_command "$name"
  resolved="$(command -v "$name")"
  case "$resolved" in
    /nix/store/*) ;;
    *)
      printf 'error: %s resolved outside the Nix store: %s\n' "$name" "$resolved" >&2
      exit 2
      ;;
  esac
}

load_local_env() {
  if [[ ! -f "$LOCAL_ENV_FILE" ]]; then
    printf '%s\n' 'error: .env.local is missing; run the task-1 local-env-create card' >&2
    exit 2
  fi

  while IFS='=' read -r key value; do
    [[ -z "$key" || "$key" == \#* ]] && continue
    if [[ ! "$key" =~ ^[A-Z][A-Z0-9_]*$ ]]; then
      printf 'error: invalid key in .env.local: %s\n' "$key" >&2
      exit 2
    fi
    export "$key=$value"
  done < "$LOCAL_ENV_FILE"
}

require_value() {
  local name="$1"
  if [[ -z "${!name:-}" ]]; then
    printf 'error: required local value is empty: %s\n' "$name" >&2
    exit 2
  fi
}

validate_compose_project() {
  if ! grep -Fxq "name: $COMPOSE_PROJECT" "$COMPOSE_FILE"; then
    printf 'error: compose project is not exactly %s\n' "$COMPOSE_PROJECT" >&2
    exit 2
  fi
}

require_compose_provider() {
  require_nix_command podman

  if [[ -z "${PODMAN_COMPOSE_PROVIDER:-}" ]]; then
    printf '%s\n' 'error: PODMAN_COMPOSE_PROVIDER is not set by the devShell' >&2
    exit 2
  fi
  case "$PODMAN_COMPOSE_PROVIDER" in
    /nix/store/*/bin/podman-compose) ;;
    *)
      printf 'error: compose provider is not the flake-pinned Nix provider: %s\n' "$PODMAN_COMPOSE_PROVIDER" >&2
      exit 2
      ;;
  esac
  if [[ ! -x "$PODMAN_COMPOSE_PROVIDER" ]]; then
    printf 'error: compose provider is not executable: %s\n' "$PODMAN_COMPOSE_PROVIDER" >&2
    exit 2
  fi
}

require_rootless_podman() {
  local rootless
  if ! rootless="$(podman info --format '{{.Host.Security.Rootless}}' 2>/dev/null)"; then
    printf '%s\n' 'error: Podman is unavailable; on macOS initialize/start podman machine yourself' >&2
    exit 2
  fi
  if [[ "$rootless" != "true" ]]; then
    printf 'error: the active Podman service is not rootless (reported %s)\n' "$rootless" >&2
    exit 2
  fi
}

compose() {
  podman compose \
    --env-file "$LOCAL_ENV_FILE" \
    --file "$COMPOSE_FILE" \
    --project-name "$COMPOSE_PROJECT" \
    "$@"
}

prepare_infra() {
  require_nix_shell
  validate_compose_project
  require_compose_provider
  load_local_env
  require_rootless_podman
}
