#!/usr/bin/env bash
set -euo pipefail

readonly active_generation='task13-r21-g0-f9-frozen-coverage-entrypoint-20260907'
readonly manifest_relative='tests/nix/task-13-assertion-manifest-r21-g0-f9.json'
readonly lock_relative='tests/nix/task-13-assertion-manifest-r21-g0-f9.sha256'
readonly record_relative='.agents/results/task-13-s2-r21-g0-f9-manifest-digest-20260907.txt'
readonly entrypoint_relative='scripts/tasks/task-13/strict-entrypoint-r21.sh'
readonly guard_relative='scripts/tasks/task-13/guarded-just-r21.sh'

manifest_copy=''
guard_copy=''

cleanup() {
  [[ -z "$manifest_copy" ]] || rm -f -- "$manifest_copy"
  [[ -z "$guard_copy" ]] || rm -f -- "$guard_copy"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

reject() {
  printf 'error: %s\n' "$1" >&2
  exit 2
}

usage() {
  printf '%s\n' 'usage: verified strict-entrypoint-r21.sh <r21-recorded-digest> (receipt | pair <justfile-relative-path> <private-recipe> [-- <recipe-args>])' >&2
  exit 2
}

sha256_file() {
  sha256sum "$1" | awk '{print $1}'
}

verify_repo_file() {
  local relative_path="$1"
  local phase="$2"
  local directory basename physical_directory physical_path
  [[ "$relative_path" =~ ^[A-Za-z0-9._/-]+$ && "$relative_path" != /* && "$relative_path" != *'..'* ]] || reject "noncanonical repository path at ${phase}"
  directory="$repo_root/$(dirname -- "$relative_path")"
  basename="$(basename -- "$relative_path")"
  [[ -d "$directory" ]] || reject "repository directory missing at ${phase}"
  physical_directory="$(cd -P -- "$directory" && pwd -P)" || reject "repository directory cannot be resolved at ${phase}"
  physical_path="${physical_directory}/${basename}"
  [[ "$physical_path" == "$repo_root/$relative_path" && -f "$physical_path" && ! -L "$physical_path" ]] || reject "repository file missing, linked, or escaped at ${phase}"
}

verify_external_read_only_copy() {
  local copy_path="$1"
  local expected_sha="$2"
  local phase="$3"
  local copy_directory physical_directory physical_copy
  [[ "$copy_path" == /* && -f "$copy_path" && ! -L "$copy_path" ]] || reject "external copy missing or linked at ${phase}"
  copy_directory="$(dirname -- "$copy_path")"
  physical_directory="$(cd -P -- "$copy_directory" && pwd -P)" || reject "external copy directory cannot be resolved at ${phase}"
  physical_copy="${physical_directory}/$(basename -- "$copy_path")"
  [[ "$physical_copy" == "$copy_path" ]] || reject "external copy path changed at ${phase}"
  case "$physical_copy" in
    "$repo_root" | "$repo_root"/*) reject "external copy is inside the repository at ${phase}" ;;
  esac
  [[ "$(stat -c '%a' -- "$physical_copy")" == 400 ]] || reject "external copy mode is not 0400 at ${phase}"
  [[ "$(sha256_file "$physical_copy")" == "$expected_sha" ]] || reject "external copy digest mismatch at ${phase}"
}

[[ -n "${IN_NIX_SHELL:-}" ]] || reject 'R21 strict entrypoint requires the already-active repository Nix devShell'
[[ "${TASK13_R21_ENTRYPOINT_SHA256:-}" =~ ^[0-9a-f]{64}$ ]] || reject 'operator entrypoint digest must be lowercase 64-hex'

readonly invocation_path="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/$(basename -- "${BASH_SOURCE[0]}")"
[[ -f "$invocation_path" && ! -L "$invocation_path" ]] || reject 'strict entrypoint invocation is missing or linked'
readonly invocation_identity="$(stat -c '%d:%i' -- "$invocation_path")"
readonly operator_entrypoint_sha="$TASK13_R21_ENTRYPOINT_SHA256"
[[ "$(stat -c '%a' -- "$invocation_path")" == 400 ]] || reject 'strict entrypoint invocation mode must be 0400'
[[ "$(sha256_file "$invocation_path")" == "$operator_entrypoint_sha" ]] || reject 'strict entrypoint self digest differs from the operator receipt'

readonly repo_root="$(pwd -P)"
[[ "$repo_root" == /* && -d "$repo_root" ]] || reject 'repository root cannot be resolved'
case "$invocation_path" in
  "$repo_root" | "$repo_root"/*) reject 'strict entrypoint must execute from an external copy' ;;
esac

(( $# >= 2 )) || usage
readonly operator_manifest_sha="$1"
readonly requested_mode="$2"
shift 2
[[ "$operator_manifest_sha" =~ ^[0-9a-f]{64}$ ]] || reject 'operator manifest digest must be lowercase 64-hex'

case "$requested_mode" in
  receipt)
    (($# == 0)) || usage
    ;;
  pair)
    (($# >= 2)) || usage
    target_justfile="$1"
    target_recipe="$2"
    shift 2
    recipe_arguments=()
    if (($# > 0)); then
      [[ "$1" == '--' ]] || reject 'recipe arguments require the literal -- separator'
      shift
      (($# > 0)) || reject 'the recipe argument separator cannot be empty'
      recipe_arguments=("$@")
    fi
    readonly target_justfile target_recipe
    readonly -a recipe_arguments
    ;;
  *) usage ;;
esac

for required_path in "$manifest_relative" "$lock_relative" "$record_relative" "$entrypoint_relative" "$guard_relative"; do
  verify_repo_file "$required_path" 'strict bootstrap material'
done

readonly live_manifest="$repo_root/$manifest_relative"
readonly live_lock="$repo_root/$lock_relative"
readonly live_record="$repo_root/$record_relative"
readonly live_entrypoint="$repo_root/$entrypoint_relative"
readonly live_guard="$repo_root/$guard_relative"

readonly manifest_identity="$(stat -c '%d:%i' -- "$live_manifest")"
[[ "$(sha256_file "$live_manifest")" == "$operator_manifest_sha" ]] || reject 'live R21 manifest digest differs from the operator receipt'
cmp -s <(jq -cS '.' "$live_manifest") "$live_manifest" || reject 'live R21 manifest is not canonical compact sorted JSON plus one LF'
cmp -s <(printf '%s\n' "$operator_manifest_sha") "$live_lock" || reject 'R21 manifest lock differs from the operator receipt'
cmp -s <(printf '%s\n' "$operator_manifest_sha") "$live_record" || reject 'R21 coordinator record differs from the operator receipt'

manifest_copy="$(mktemp "${TMPDIR:-/tmp}/jamye-task13-r21-manifest.XXXXXX")" || reject 'cannot create external R21 manifest copy'
cp -- "$live_manifest" "$manifest_copy" || reject 'cannot copy the R21 manifest'
chmod 0400 -- "$manifest_copy" || reject 'cannot make the R21 manifest copy read-only'
manifest_copy="$(cd -P -- "$(dirname -- "$manifest_copy")" && pwd -P)/$(basename -- "$manifest_copy")"
verify_external_read_only_copy "$manifest_copy" "$operator_manifest_sha" 'manifest-copy'
cmp -s <(jq -cS '.' "$manifest_copy") "$manifest_copy" || reject 'R21 manifest copy is not canonical compact sorted JSON plus one LF'

jq -e --arg generation "$active_generation" --arg entrypoint "$entrypoint_relative" --arg guard "$guard_relative" '
  .generation_id == $generation
  and .authority_binding.generation_id == $generation
  and .guarded_dispatch.generation_id == $generation
  and .guarded_dispatch.adapter == $guard
  and .strict_entrypoint.path == $entrypoint
  and (.command_source_integrity.canonical_order | type == "array")
  and ([.command_source_integrity.sources[] | select(.path == $entrypoint)] | length == 1)
  and ([.command_source_integrity.sources[] | select(.path == $guard)] | length == 1)
' "$manifest_copy" >/dev/null || reject 'R21 manifest bootstrap projection is invalid'

readonly manifest_entrypoint_sha="$(jq -er --arg path "$entrypoint_relative" '.command_source_integrity.sources[] | select(.path == $path) | .sha256' "$manifest_copy")"
readonly manifest_guard_sha="$(jq -er --arg path "$guard_relative" '.command_source_integrity.sources[] | select(.path == $path) | .sha256' "$manifest_copy")"
[[ "$manifest_entrypoint_sha" =~ ^[0-9a-f]{64}$ && "$manifest_guard_sha" =~ ^[0-9a-f]{64}$ ]] || reject 'R21 manifest bootstrap source digest is malformed'
[[ "$manifest_entrypoint_sha" == "$operator_entrypoint_sha" ]] || reject 'manifest-bound strict entrypoint digest differs from the operator receipt'
[[ "$(sha256_file "$live_entrypoint")" == "$manifest_entrypoint_sha" ]] || reject 'live strict entrypoint differs from the manifest-bound source'
[[ "$(sha256_file "$live_guard")" == "$manifest_guard_sha" ]] || reject 'live R21 guard differs from the manifest-bound source'

readonly guard_identity="$(stat -c '%d:%i' -- "$live_guard")"
guard_copy="$(mktemp "${TMPDIR:-/tmp}/jamye-task13-r21-guard.XXXXXX")" || reject 'cannot create external R21 guard copy'
cp -- "$live_guard" "$guard_copy" || reject 'cannot copy the R21 guard'
chmod 0400 -- "$guard_copy" || reject 'cannot make the R21 guard copy read-only'
guard_copy="$(cd -P -- "$(dirname -- "$guard_copy")" && pwd -P)/$(basename -- "$guard_copy")"
verify_external_read_only_copy "$guard_copy" "$manifest_guard_sha" 'guard-copy'

# Revalidate every operator-rooted byte and physical identity immediately before
# handing authority to the copied guard.
[[ "$(stat -c '%d:%i' -- "$invocation_path")" == "$invocation_identity" ]] || reject 'strict entrypoint copy identity changed before guard dispatch'
verify_external_read_only_copy "$invocation_path" "$operator_entrypoint_sha" 'entrypoint-final'
[[ "$(stat -c '%d:%i' -- "$live_manifest")" == "$manifest_identity" ]] || reject 'live R21 manifest identity changed during strict bootstrap'
[[ "$(sha256_file "$live_manifest")" == "$operator_manifest_sha" ]] || reject 'live R21 manifest changed during strict bootstrap'
cmp -s "$manifest_copy" "$live_manifest" || reject 'live R21 manifest differs from the verified copy'
cmp -s <(printf '%s\n' "$operator_manifest_sha") "$live_lock" || reject 'R21 manifest lock changed during strict bootstrap'
cmp -s <(printf '%s\n' "$operator_manifest_sha") "$live_record" || reject 'R21 coordinator record changed during strict bootstrap'
[[ "$(stat -c '%d:%i' -- "$live_guard")" == "$guard_identity" ]] || reject 'live R21 guard identity changed during strict bootstrap'
[[ "$(sha256_file "$live_guard")" == "$manifest_guard_sha" ]] || reject 'live R21 guard changed during strict bootstrap'
verify_external_read_only_copy "$guard_copy" "$manifest_guard_sha" 'guard-copy-final'

export TASK13_R21_REPO_ROOT="$repo_root"
export TASK13_R21_MANIFEST_COPY="$manifest_copy"
export TASK13_R21_OPERATOR_MANIFEST_SHA256="$operator_manifest_sha"
export TASK13_R21_STRICT_ENTRYPOINT_COPY="$invocation_path"
export TASK13_R21_STRICT_ENTRYPOINT_SHA256="$operator_entrypoint_sha"
export TASK13_R21_GUARD_COPY="$guard_copy"
export TASK13_R21_GUARD_SHA256="$manifest_guard_sha"

set +e
if [[ "$requested_mode" == 'receipt' ]]; then
  bash "$guard_copy" "$active_generation" --receipt
  nested_status=$?
else
  guard_argv=("$guard_copy" "$active_generation" "$operator_manifest_sha" pair "$target_justfile" "$target_recipe")
  if ((${#recipe_arguments[@]} > 0)); then
    guard_argv+=(-- "${recipe_arguments[@]}")
  fi
  bash "${guard_argv[@]}"
  nested_status=$?
fi
set -e
exit "$nested_status"
