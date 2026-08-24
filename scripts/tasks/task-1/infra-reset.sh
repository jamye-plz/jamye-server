#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

prepare_infra

if [[ "${JAMYE_CONFIRM_INFRA_RESET:-}" != "$COMPOSE_PROJECT" ]]; then
  printf 'error: set JAMYE_CONFIRM_INFRA_RESET=%s to confirm disposable-byte deletion\n' "$COMPOSE_PROJECT" >&2
  exit 2
fi

expected_volumes=(
  jamye-server-test-postgres-data
  jamye-server-test-redis-data
  jamye-server-test-minio-data
)

compose down --remove-orphans

for volume in "${expected_volumes[@]}"; do
  if [[ "$volume" != "$COMPOSE_PROJECT"-* ]]; then
    printf 'error: unsafe volume name: %s\n' "$volume" >&2
    exit 2
  fi
  if ! podman volume exists "$volume"; then
    continue
  fi

  compose_label="$(podman volume inspect --format '{{index .Labels "com.docker.compose.project"}}' "$volume" 2>/dev/null || true)"
  podman_label="$(podman volume inspect --format '{{index .Labels "io.podman.compose.project"}}' "$volume" 2>/dev/null || true)"
  if [[ "$compose_label" != "$COMPOSE_PROJECT" && "$podman_label" != "$COMPOSE_PROJECT" ]]; then
    printf 'error: volume %s lacks the expected project ownership label; refusing reset\n' "$volume" >&2
    exit 2
  fi
done

for volume in "${expected_volumes[@]}"; do
  if podman volume exists "$volume"; then
    podman volume rm "$volume"
  fi
done

printf '%s\n' 'deleted only the three guarded jamye-server-test named volumes'
printf '%s\n' '.env.local was preserved; delete it separately only if you intend to rotate local credentials'
