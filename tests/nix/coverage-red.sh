#!/usr/bin/env bash
set -euo pipefail

readonly generation_id='task13-r15-g0-f3-coverage-authority-20260905'
readonly task_justfile="${TASK13_BOUND_JUSTFILE:-}"
readonly task_justfile_identity="${TASK13_BOUND_JUSTFILE_IDENTITY:-}"
readonly task_justfile_sha="${TASK13_BOUND_JUSTFILE_SHA256:-}"
readonly assertion_copy="${TASK13_COVERAGE_RED_ASSERTION:-}"
readonly assertion_copy_identity="${TASK13_COVERAGE_RED_ASSERTION_IDENTITY:-}"
readonly assertion_copy_sha="${TASK13_COVERAGE_RED_ASSERTION_SHA256:-}"

invalid() {
  printf 'INVALID: %s\n' "$1" >&2
  exit 2
}

[[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation_id" ]] || invalid 'guarded R15 generation receipt missing'
[[ "${TASK13_ASSERTION_DIGEST:-}" =~ ^[0-9a-f]{64}$ ]] || invalid 'guarded R15 digest receipt missing'

verify_bound_copy() {
  local path="$1"
  local expected_identity="$2"
  local expected_sha="$3"
  local label="$4"
  local directory basename physical_directory physical_path
  [[ "$path" == /* && "$expected_identity" =~ ^[0-9]+:[0-9]+$ && "$expected_sha" =~ ^[0-9a-f]{64}$ ]] || invalid "${label} receipt is malformed"
  directory="$(dirname -- "$path")"
  basename="$(basename -- "$path")"
  [[ -d "$directory" ]] || invalid "${label} directory is missing"
  physical_directory="$(cd -P -- "$directory" && pwd -P)" || invalid "${label} directory cannot be resolved"
  physical_path="${physical_directory}/${basename}"
  [[ "$physical_path" == "$path" && -f "$physical_path" && ! -L "$physical_path" ]] || invalid "${label} is missing, linked, or escaped"
  [[ "$(stat -c '%d:%i' -- "$physical_path")" == "$expected_identity" ]] || invalid "${label} identity changed"
  [[ "$(sha256sum "$physical_path" | awk '{print $1}')" == "$expected_sha" ]] || invalid "${label} content changed"
}

verify_bound_copy "$task_justfile" "$task_justfile_identity" "$task_justfile_sha" 'bound Task-13 Justfile copy'
verify_bound_copy "$assertion_copy" "$assertion_copy_identity" "$assertion_copy_sha" 'bound coverage RED assertion copy'
[[ "$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/$(basename -- "${BASH_SOURCE[0]}")" == "$assertion_copy" ]] || invalid 'coverage RED assertion did not execute from the guard-bound copy'

recipe_header_count() {
  local recipe_name="$1"
  awk -v recipe_name="$recipe_name" '
    {
      if ($0 ~ /^[[:space:]]/) next
      line = $0
      if (line == "" || line ~ /^#/ || line ~ /^\[/) next
      if (substr(line, 1, length(recipe_name)) != recipe_name) next
      boundary = substr(line, length(recipe_name) + 1, 1)
      if (boundary != ":" && boundary !~ /[[:space:]]/) next
      remainder = substr(line, length(recipe_name) + 1)
      if (remainder ~ /^[[:space:]]*:=/) next
      if (index(remainder, ":") > 0) count++
    }
    END { print count + 0 }
  ' "$task_justfile"
}

[[ "$(recipe_header_count 'coverage-red')" == 1 ]] || invalid 'public coverage-red transport is not exact'
[[ "$(recipe_header_count 'coverage-red-r15-exec')" == 1 ]] || invalid 'private coverage RED executor is not exact'

public_green_count="$(recipe_header_count 'coverage')"
private_green_count="$(recipe_header_count 'coverage-r15-exec')"
[[ "$public_green_count" == 0 && "$private_green_count" == 0 ]] || invalid 'GREEN coverage recipe already exists; RED staging is closed'

if command -v cargo-llvm-cov >/dev/null 2>&1; then
  invalid 'cargo-llvm-cov is already resolvable; RED staging is closed'
fi

printf '%s\n' 'FAIL: coverage recipe missing'
printf '%s\n' 'FAIL: cargo-llvm-cov devShell tool missing'
printf '%s\n' 'SUMMARY: 2 FAIL / 0 PASS / 0 SKIP'
printf '%s\n' 'DISPOSITION: BASELINE_RED (two named failures)'
exit 1
