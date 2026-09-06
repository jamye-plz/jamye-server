#!/usr/bin/env bash
set -euo pipefail

# R25 bootstrap: this program is intentionally executed only from a 0400
# external copy.  It owns the manifest and guard copies; the copied guard owns
# every selected Justfile and helper copy.
readonly generation_id='task13-r25-g0-f13-trust-root-cross-binding-20260907'
readonly manifest_relative='tests/nix/task-13-assertion-manifest-r25-g0-f13.json'
readonly lock_relative='tests/nix/task-13-assertion-manifest-r25-g0-f13.sha256'
readonly record_relative='.agents/results/task-13-s2-r25-g0-f13-manifest-digest-20260907.txt'
readonly entrypoint_relative='scripts/tasks/task-13/strict-entrypoint-r25.sh'
readonly guard_relative='scripts/tasks/task-13/guarded-just-r25.sh'

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

reject() { printf 'error: %s\n' "$1" >&2; exit 2; }
usage() {
  printf '%s\n' 'usage: verified strict-entrypoint-r25.sh <manifest-digest> (receipt | pair <justfile-relative-path> <private-recipe> [-- <recipe-args>])' >&2
  exit 2
}
sha256_file() { sha256sum "$1" | awk '{print $1}'; }
identity() { stat -c '%d:%i' -- "$1"; }

verify_repo_file() {
  local relative_path="$1" phase="$2" directory physical_directory physical_path
  [[ "$relative_path" =~ ^[A-Za-z0-9._/-]+$ && "$relative_path" != /* && "$relative_path" != *'..'* ]] || reject "noncanonical repository path at ${phase}"
  directory="$repo_root/$(dirname -- "$relative_path")"
  [[ -d "$directory" ]] || reject "repository directory missing at ${phase}"
  physical_directory="$(cd -P -- "$directory" && pwd -P)" || reject "repository directory cannot be resolved at ${phase}"
  physical_path="${physical_directory}/$(basename -- "$relative_path")"
  [[ "$physical_path" == "$repo_root/$relative_path" && -f "$physical_path" && ! -L "$physical_path" ]] || reject "repository file missing, linked, or escaped at ${phase}"
}

verify_external_copy() {
  local path="$1" expected_sha="$2" phase="$3" directory physical_directory physical_path
  [[ "$expected_sha" =~ ^[0-9a-f]{64}$ ]] || reject "external-copy hash is malformed at ${phase}"
  [[ "$path" == /* && -f "$path" && ! -L "$path" ]] || reject "external copy missing or linked at ${phase}"
  directory="$(dirname -- "$path")"
  physical_directory="$(cd -P -- "$directory" && pwd -P)" || reject "external copy directory cannot be resolved at ${phase}"
  physical_path="${physical_directory}/$(basename -- "$path")"
  [[ "$physical_path" == "$path" ]] || reject "external copy path changed at ${phase}"
  case "$physical_path" in "$repo_root"|"$repo_root"/*) reject "external copy is inside the repository at ${phase}";; esac
  [[ "$(stat -c '%a' -- "$physical_path")" == 400 ]] || reject "external copy mode is not 0400 at ${phase}"
  [[ "$(sha256_file "$physical_path")" == "$expected_sha" ]] || reject "external copy digest mismatch at ${phase}"
}

[[ -n "${IN_NIX_SHELL:-}" ]] || reject 'R25 strict entrypoint requires the already-active repository Nix devShell'
[[ "${TASK13_R25_ENTRYPOINT_SHA256:-}" =~ ^[0-9a-f]{64}$ ]] || reject 'operator entrypoint digest must be lowercase 64-hex'
readonly repo_root="$(pwd -P)"
[[ "$repo_root" == /* && -d "$repo_root" ]] || reject 'repository root cannot be resolved'
readonly invocation_path="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/$(basename -- "${BASH_SOURCE[0]}")"
case "$invocation_path" in "$repo_root"|"$repo_root"/*) reject 'strict entrypoint must execute from an external copy';; esac
readonly operator_entrypoint_sha="$TASK13_R25_ENTRYPOINT_SHA256"
verify_external_copy "$invocation_path" "$operator_entrypoint_sha" 'entrypoint-bootstrap'
readonly invocation_identity="$(identity "$invocation_path")"

(( $# >= 2 )) || usage
readonly operator_manifest_sha="$1"
readonly requested_mode="$2"
shift 2
[[ "$operator_manifest_sha" =~ ^[0-9a-f]{64}$ ]] || reject 'operator manifest digest must be lowercase 64-hex'
case "$requested_mode" in
  receipt) (($# == 0)) || usage ;;
  pair)
    (($# >= 2)) || usage
    target_justfile="$1"; target_recipe="$2"; shift 2
    recipe_arguments=()
    if (($#)); then [[ "$1" == '--' ]] || reject 'recipe arguments require the literal -- separator'; shift; (($#)) || reject 'the recipe argument separator cannot be empty'; recipe_arguments=("$@"); fi
    readonly target_justfile target_recipe
    readonly -a recipe_arguments
    ;;
  *) usage ;;
esac

for path in "$manifest_relative" "$lock_relative" "$record_relative" "$entrypoint_relative" "$guard_relative"; do
  verify_repo_file "$path" 'strict bootstrap material'
done
readonly live_manifest="$repo_root/$manifest_relative"
readonly live_lock="$repo_root/$lock_relative"
readonly live_record="$repo_root/$record_relative"
readonly live_guard="$repo_root/$guard_relative"
readonly manifest_identity="$(identity "$live_manifest")"
[[ "$(sha256_file "$live_manifest")" == "$operator_manifest_sha" ]] || reject 'live R25 manifest digest differs from the operator receipt'
cmp -s <(jq -cS '.' "$live_manifest") "$live_manifest" || reject 'live R25 manifest is not canonical compact sorted JSON plus one LF'
cmp -s <(printf '%s\n' "$operator_manifest_sha") "$live_lock" || reject 'R25 manifest lock differs from the operator receipt'
cmp -s <(printf '%s\n' "$operator_manifest_sha") "$live_record" || reject 'R25 coordinator record differs from the operator receipt'

manifest_copy="$(mktemp "${TMPDIR:-/tmp}/jamye-task13-r25-manifest.XXXXXX")" || reject 'cannot create external R25 manifest copy'
cp -- "$live_manifest" "$manifest_copy" || reject 'cannot copy R25 manifest'
chmod 0400 -- "$manifest_copy" || reject 'cannot make R25 manifest copy read-only'
manifest_copy="$(cd -P -- "$(dirname -- "$manifest_copy")" && pwd -P)/$(basename -- "$manifest_copy")"
verify_external_copy "$manifest_copy" "$operator_manifest_sha" 'manifest-copy'

# Validate the complete bootstrap projection before using any manifest-derived
# path.  The exact source-row count and type are repeated below for the trust
# root binding itself.
jq -e --arg generation "$generation_id" --arg entrypoint "$entrypoint_relative" --arg guard "$guard_relative" '
  (keys | sort) == ["aarch64_linux_builder","authority_binding","builder","command_source_integrity","coverage","cross_system","deployment_boundary","dynamic_hash_rule","env_timing_boundary","final_tree_record_protocol","final_tree_snapshot","flake_shape","generation_id","guarded_dispatch","minio_interface_compatibility","module_eval","module_package_provenance","operator_surfaces","strict_entrypoint","token_schema"]
  and .generation_id == $generation
  and .authority_binding.generation_id == $generation
  and .authority_binding.runtime_resolution == "copied_manifest_only"
  and .strict_entrypoint.path == $entrypoint
  and .strict_entrypoint.operator_environment_digest == "TASK13_R25_ENTRYPOINT_SHA256"
  and .guarded_dispatch.adapter == $guard
  and ((.guarded_dispatch.allowed_pairs | type) == "array")
  and ((.guarded_dispatch.allowed_pairs | length) == 24)
  and ((.guarded_dispatch.allowed_pairs | unique | length) == 24)
  and (.command_source_integrity.canonical_order | type == "array" and length == 8)
  and (.command_source_integrity.sources | type == "array" and length == 8)
  and ([.command_source_integrity.sources[] | select(.path == $entrypoint)] | length == 1)
  and ([.command_source_integrity.sources[] | select(.path == $guard)] | length == 1)
  and (.token_schema | type == "object")
' "$manifest_copy" >/dev/null || reject 'R25 copied manifest bootstrap projection is invalid'

# This is deliberately before guard copying, receipt processing, pair
# selection, helper execution, and every nested action.
readonly manifest_entrypoint_sha="$(jq -er --arg path "$entrypoint_relative" '[.command_source_integrity.sources[] | select(.path == $path)] | if length == 1 and .[0].type == "regular_non_symlink" and (.[0].sha256 | test("^[0-9a-f]{64}$")) then .[0].sha256 else error("invalid strict entrypoint source row") end' "$manifest_copy")" || reject 'R25 strict-entrypoint manifest source row is invalid'
[[ "$manifest_entrypoint_sha" == "$operator_entrypoint_sha" ]] || reject 'manifest strict-entrypoint hash differs from operator digest'
[[ "$(identity "$invocation_path")" == "$invocation_identity" ]] || reject 'strict entrypoint copy identity changed before guard copy'
verify_external_copy "$invocation_path" "$manifest_entrypoint_sha" 'entrypoint-self-manifest-binding'
readonly manifest_guard_sha="$(jq -er --arg path "$guard_relative" '[.command_source_integrity.sources[] | select(.path == $path)] | if length == 1 and .[0].type == "regular_non_symlink" and (.[0].sha256 | test("^[0-9a-f]{64}$")) then .[0].sha256 else error("invalid guard source row") end' "$manifest_copy")" || reject 'R25 guard manifest source row is invalid'
[[ "$(sha256_file "$live_guard")" == "$manifest_guard_sha" ]] || reject 'live R25 guard differs from the manifest-bound source'

guard_copy="$(mktemp "${TMPDIR:-/tmp}/jamye-task13-r25-guard.XXXXXX")" || reject 'cannot create external R25 guard copy'
cp -- "$live_guard" "$guard_copy" || reject 'cannot copy R25 guard'
chmod 0400 -- "$guard_copy" || reject 'cannot make R25 guard copy read-only'
guard_copy="$(cd -P -- "$(dirname -- "$guard_copy")" && pwd -P)/$(basename -- "$guard_copy")"
verify_external_copy "$guard_copy" "$manifest_guard_sha" 'guard-copy'

[[ "$(identity "$live_manifest")" == "$manifest_identity" ]] || reject 'live R25 manifest identity changed during strict bootstrap'
[[ "$(sha256_file "$live_manifest")" == "$operator_manifest_sha" ]] || reject 'live R25 manifest changed during strict bootstrap'
cmp -s "$manifest_copy" "$live_manifest" || reject 'live R25 manifest differs from copied manifest'
cmp -s <(printf '%s\n' "$operator_manifest_sha") "$live_lock" || reject 'R25 manifest lock changed during strict bootstrap'
cmp -s <(printf '%s\n' "$operator_manifest_sha") "$live_record" || reject 'R25 record changed during strict bootstrap'
verify_external_copy "$invocation_path" "$manifest_entrypoint_sha" 'entrypoint-final'
verify_external_copy "$guard_copy" "$manifest_guard_sha" 'guard-copy-final'

export TASK13_R25_REPO_ROOT="$repo_root"
export TASK13_R25_MANIFEST_COPY="$manifest_copy"
export TASK13_R25_OPERATOR_MANIFEST_SHA256="$operator_manifest_sha"
export TASK13_R25_STRICT_ENTRYPOINT_COPY="$invocation_path"
export TASK13_R25_STRICT_ENTRYPOINT_SHA256="$manifest_entrypoint_sha"
export TASK13_R25_GUARD_COPY="$guard_copy"
export TASK13_R25_GUARD_SHA256="$manifest_guard_sha"
set +e
if [[ "$requested_mode" == receipt ]]; then
  bash "$guard_copy" "$generation_id" --receipt
else
  argv=("$guard_copy" "$generation_id" "$operator_manifest_sha" pair "$target_justfile" "$target_recipe")
  ((${#recipe_arguments[@]} == 0)) || argv+=(-- "${recipe_arguments[@]}")
  bash "${argv[@]}"
fi
status=$?
set -e
exit "$status"
