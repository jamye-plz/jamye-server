#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

require_nix_shell
load_local_env
require_nix_command curl
require_nix_command jq

base_url="http://$JAMYE_LISTEN_ADDR"
live_response="$(curl --fail --silent --show-error "$base_url/health/live")"
ready_response="$(curl --fail --silent --show-error "$base_url/health/ready")"

jq -e '.status == "live"' <<<"$live_response" >/dev/null
jq -e '
  .status == "ready"
  and .checks.postgres.status == "ready"
  and .checks.redis.status == "ready"
  and .checks.minio.status == "ready"
' <<<"$ready_response" >/dev/null

jq . <<<"$live_response"
jq . <<<"$ready_response"
