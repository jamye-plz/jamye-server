#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

require_nix_shell
require_nix_command gitleaks
cd "$REPO_ROOT"

if [[ -e "$REPO_ROOT/.env.local" ]]; then
  printf '%s\n' 'error: .env.local contains intentional disposable credentials' >&2
  printf '%s\n' 'run this M0 scanner self-test before local-env-create; do not hide the file inside the scan' >&2
  exit 2
fi

sentinel="$REPO_ROOT/.gitleaks-task-1-sentinel.txt"
if [[ -e "$sentinel" ]]; then
  printf 'error: sentinel path already exists: %s\n' "$sentinel" >&2
  exit 2
fi
trap 'rm -f -- "$sentinel"' EXIT

gitleaks dir . --config "$REPO_ROOT/.gitleaks.toml" --no-banner --redact

part_a='gh'
part_b='p_'
part_c='a9B8c7D6e5F4g3H2i1'
part_d='J0k9L8m7N6o5P4q3R2'
printf 'synthetic_test_token = "%s%s%s%s"\n' "$part_a" "$part_b" "$part_c" "$part_d" > "$sentinel"

if gitleaks dir . --config "$REPO_ROOT/.gitleaks.toml" --no-banner --redact; then
  printf '%s\n' 'error: gitleaks did not detect the synthetic untracked sentinel' >&2
  exit 1
fi

rm -f -- "$sentinel"
gitleaks dir . --config "$REPO_ROOT/.gitleaks.toml" --no-banner --redact
trap - EXIT

printf '%s\n' 'gitleaks clean/detect/clean regression passed'
