#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

prepare_infra
compose down --remove-orphans
printf '%s\n' 'local containers and project network stopped; named-volume bytes were preserved'
