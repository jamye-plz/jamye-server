#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/common.sh"

require_nix_shell
require_nix_command gitleaks
cd "$REPO_ROOT"

scan_root="$(mktemp -d "${TMPDIR:-/tmp}/jamye-secret-scan.XXXXXX")"
trap 'rm -rf -- "$scan_root"' EXIT

while IFS= read -r -d '' relative_path; do
  if [[ ! -f "$relative_path" && ! -L "$relative_path" ]]; then
    continue
  fi
  destination="$scan_root/$relative_path"
  mkdir -p -- "$(dirname -- "$destination")"
  cp -P -- "$relative_path" "$destination"
done < <(git ls-files --cached --others --exclude-standard -z)

gitleaks dir "$scan_root" \
  --config "$REPO_ROOT/.gitleaks.toml" \
  --no-banner \
  --redact

printf '%s\n' 'tracked and unignored working-tree content contains no detected secrets'
