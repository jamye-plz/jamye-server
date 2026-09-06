#!/usr/bin/env bash
set -euo pipefail

# The R25 guard is the sole owner of selected Justfile and helper copies.
readonly generation_id='task13-r25-g0-f13-trust-root-cross-binding-20260907'
readonly shape_relative='scripts/tasks/task-13/r25-shape.just'
readonly snapshot_helper_relative='scripts/tasks/task-13/flake-source-snapshot-r25.sh'
readonly final_tree_helper_relative='scripts/tasks/task-13/final-tree-record-r25.sh'
readonly final_verify_helper_relative='scripts/tasks/task-13/final-verify-r25.sh'
readonly standalone_ere='^[[:space:]]*(import|import\?|mod|mod\?)[[:space:]]+'

selected_copy=''
snapshot_helper_copy=''
final_tree_helper_copy=''
final_verify_helper_copy=''
cleanup() {
  [[ -z "$selected_copy" ]] || rm -f -- "$selected_copy"
  [[ -z "$snapshot_helper_copy" ]] || rm -f -- "$snapshot_helper_copy"
  [[ -z "$final_tree_helper_copy" ]] || rm -f -- "$final_tree_helper_copy"
  [[ -z "$final_verify_helper_copy" ]] || rm -f -- "$final_verify_helper_copy"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

reject() { printf 'error: %s\n' "$1" >&2; exit 2; }
usage() {
  printf '%s\n' 'usage: copied guarded-just-r25.sh task13-r25-g0-f13-trust-root-cross-binding-20260907 (--receipt | <manifest-digest> pair <justfile-relative-path> <private-recipe> [-- <recipe-args>])' >&2
  exit 2
}
sha256_file() { sha256sum "$1" | awk '{print $1}'; }
identity() { stat -c '%d:%i' -- "$1"; }

[[ -n "${IN_NIX_SHELL:-}" ]] || reject 'R25 guard requires the already-active repository Nix devShell'
[[ "${TASK13_R25_REPO_ROOT:-}" == /* && -d "${TASK13_R25_REPO_ROOT:-}" ]] || reject 'strict-entrypoint repository-root receipt is missing'
readonly repo_root="$(cd -P -- "$TASK13_R25_REPO_ROOT" && pwd -P)"
[[ "$(pwd -P)" == "$repo_root" ]] || reject 'R25 guard must run from the verified physical repository root'
readonly manifest_copy="${TASK13_R25_MANIFEST_COPY:-}"
readonly manifest_sha="${TASK13_R25_OPERATOR_MANIFEST_SHA256:-}"
readonly strict_copy="${TASK13_R25_STRICT_ENTRYPOINT_COPY:-}"
readonly strict_sha="${TASK13_R25_STRICT_ENTRYPOINT_SHA256:-}"
readonly guard_copy="${TASK13_R25_GUARD_COPY:-}"
readonly guard_sha="${TASK13_R25_GUARD_SHA256:-}"
[[ "$manifest_sha" =~ ^[0-9a-f]{64}$ && "$strict_sha" =~ ^[0-9a-f]{64}$ && "$guard_sha" =~ ^[0-9a-f]{64}$ ]] || reject 'strict bootstrap digest receipt is malformed'
readonly guard_invocation="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/$(basename -- "${BASH_SOURCE[0]}")"

verify_external() {
  local path="$1" expected="$2" phase="$3" directory physical_directory physical_path
  [[ "$path" == /* && -f "$path" && ! -L "$path" && "$expected" =~ ^[0-9a-f]{64}$ ]] || reject "external file missing, linked, or unhashed at ${phase}"
  directory="$(dirname -- "$path")"
  physical_directory="$(cd -P -- "$directory" && pwd -P)" || reject "external directory cannot be resolved at ${phase}"
  physical_path="${physical_directory}/$(basename -- "$path")"
  [[ "$physical_path" == "$path" ]] || reject "external path changed at ${phase}"
  case "$physical_path" in "$repo_root"|"$repo_root"/*) reject "external file is inside repository at ${phase}";; esac
  [[ "$(stat -c '%a' -- "$physical_path")" == 400 ]] || reject "external mode is not 0400 at ${phase}"
  [[ "$(sha256_file "$physical_path")" == "$expected" ]] || reject "external digest mismatch at ${phase}"
}
verify_repo() {
  local relative="$1" phase="$2" directory physical_directory physical
  [[ "$relative" =~ ^[A-Za-z0-9._/-]+$ && "$relative" != /* && "$relative" != *'..'* ]] || reject "noncanonical source path at ${phase}"
  directory="$repo_root/$(dirname -- "$relative")"
  [[ -d "$directory" ]] || reject "source directory missing at ${phase}"
  physical_directory="$(cd -P -- "$directory" && pwd -P)" || reject "source directory cannot be resolved at ${phase}"
  physical="${physical_directory}/$(basename -- "$relative")"
  [[ "$physical" == "$repo_root/$relative" && -f "$physical" && ! -L "$physical" ]] || reject "source missing, linked, or escaped at ${phase}"
}
manifest_source_sha() {
  local path="$1"
  jq -er --arg path "$path" '[.command_source_integrity.sources[] | select(.path == $path)] | if length == 1 and .[0].type == "regular_non_symlink" and (.[0].sha256 | test("^[0-9a-f]{64}$")) then .[0].sha256 else error("invalid source") end' "$manifest_copy"
}
assert_no_just_imports() {
  local source="$1" executable="$2" phase="$3"
  LC_ALL=C rg -n -- "$standalone_ere" "$source" "$executable" >/dev/null && reject "selected Justfile is not standalone at ${phase}"
}
copy_external() {
  local source="$1" expected="$2" label="$3" destination
  [[ "$(sha256_file "$source")" == "$expected" ]] || reject "source digest mismatch before ${label} copy"
  destination="$(mktemp "${TMPDIR:-/tmp}/jamye-task13-r25-${label}.XXXXXX")" || reject "cannot create ${label} copy"
  cp -- "$source" "$destination" || reject "cannot copy ${label}"
  chmod 0400 -- "$destination" || reject "cannot make ${label} copy read-only"
  destination="$(cd -P -- "$(dirname -- "$destination")" && pwd -P)/$(basename -- "$destination")"
  verify_external "$destination" "$expected" "${label}-copy"
  COPIED_PATH="$destination"
  COPIED_SHA="$expected"
}
copy_adjacent() {
  local source="$1" expected="$2" destination
  [[ "$(sha256_file "$source")" == "$expected" ]] || reject 'selected live Justfile changed before copy'
  destination="$(mktemp "$(dirname -- "$source")/.jamye-task13-r25-justfile.XXXXXX")" || reject 'cannot create adjacent selected Justfile copy'
  cp -- "$source" "$destination" || reject 'cannot copy selected Justfile'
  chmod 0400 -- "$destination" || reject 'cannot make selected Justfile copy read-only'
  destination="$(cd -P -- "$(dirname -- "$destination")" && pwd -P)/$(basename -- "$destination")"
  [[ -f "$destination" && ! -L "$destination" && "$(stat -c '%a' -- "$destination")" == 400 && "$(sha256_file "$destination")" == "$expected" ]] || reject 'adjacent selected Justfile copy differs'
  COPIED_PATH="$destination"
}
copy_helper() {
  local relative="$1" label="$2" expected source
  verify_repo "$relative" "${label}-source"
  expected="$(manifest_source_sha "$relative")" || reject "manifest helper digest is missing for ${relative}"
  source="$repo_root/$relative"
  copy_external "$source" "$expected" "$label"
}

verify_external "$manifest_copy" "$manifest_sha" 'manifest-receipt'
verify_external "$strict_copy" "$strict_sha" 'strict-entrypoint-receipt'
[[ "$guard_invocation" == "$guard_copy" ]] || reject 'guard invocation is not the copied guard selected by strict entrypoint'
verify_external "$guard_invocation" "$guard_sha" 'guard-self-receipt'
readonly manifest_identity="$(identity "$manifest_copy")"
readonly strict_identity="$(identity "$strict_copy")"
readonly guard_identity="$(identity "$guard_invocation")"
cmp -s <(jq -cS '.' "$manifest_copy") "$manifest_copy" || reject 'copied R25 manifest is not canonical compact sorted JSON plus one LF'

# Manifest is the only runtime authority.  Validate its dispatch partition and
# frozen source list before it can select a source or helper path.
jq -e --arg generation "$generation_id" --arg shape "$shape_relative" '
  def allowed: [
    "scripts/tasks/task-13/r25-shape.just::aarch64-linux-shape-r25-immutable",
    "scripts/tasks/task-1/mod.just::flake-linux",
    "scripts/tasks/task-1/mod.just::flake-local",
    "scripts/tasks/task-1/mod.just::dependency-check",
    "scripts/tasks/task-1/mod.just::platform-check",
    "scripts/tasks/task-1/mod.just::secret-scan",
    "scripts/tasks/task-11/mod.just::push-barriers-green",
    "scripts/tasks/task-12/mod.just::composition-green",
    "scripts/tasks/task-12/mod.just::contract-green",
    "scripts/tasks/task-12/mod.just::migration-chain-green",
    "scripts/tasks/task-3b/mod.just::check",
    "scripts/tasks/task-4a/mod.just::postgres-recovery",
    "scripts/tasks/task-4b/mod.just::redis-publish-config-green",
    "scripts/tasks/task-4b/mod.just::redis-recovery",
    "scripts/tasks/task-8/mod.just::contract-green",
    "scripts/tasks/task-8/mod.just::resilience-green",
    "scripts/tasks/task-9/mod.just::delivery-lifecycle-green",
    "scripts/tasks/task-9/mod.just::privacy-mutations-green",
    "scripts/tasks/task-13/r25-shape.just::coverage-r25-exec",
    "scripts/tasks/task-13/r25-shape.just::module-eval-r25-exec",
    "scripts/tasks/task-13/r25-shape.just::linux-builder-r25-exec",
    "scripts/tasks/task-13/r25-shape.just::aarch64-linux-builder-r25-exec",
    "scripts/tasks/task-13/r25-shape.just::cross-system-verify-r25-exec",
    "scripts/tasks/task-13/r25-shape.just::final-verify-r25-exec"
  ];
  .generation_id == $generation
  and .authority_binding.generation_id == $generation
  and .authority_binding.runtime_resolution == "copied_manifest_only"
  and .guarded_dispatch.adapter == "scripts/tasks/task-13/guarded-just-r25.sh"
  and .guarded_dispatch.allowed_pairs == allowed
  and .guarded_dispatch.execution_classes.immutable_snapshot == allowed[0:3]
  and .guarded_dispatch.execution_classes.live_worktree == allowed[3:24]
  and ([.command_source_integrity.sources[].path] == .command_source_integrity.canonical_order)
  and (.command_source_integrity.sources | length == 8)
  and ([.command_source_integrity.sources[] | select(.path == $shape)] | length == 1)
' "$manifest_copy" >/dev/null || reject 'copied R25 manifest dispatch/source protocol is invalid'

(( $# == 2 || $# >= 5 )) || usage
[[ "$1" == "$generation_id" ]] || reject 'unknown Task-13 R25 assertion generation'
if (($# == 2)); then
  [[ "$2" == --receipt ]] || usage
  mode='receipt'
else
  recorded_digest="$2"; [[ "$3" == pair ]] || usage; target_justfile="$4"; target_recipe="$5"; shift 5
  recipe_arguments=()
  if (($#)); then [[ "$1" == -- ]] || reject 'recipe arguments require literal -- separator'; shift; (($#)) || reject 'recipe argument separator cannot be empty'; recipe_arguments=("$@"); fi
  [[ "$recorded_digest" == "$manifest_sha" && "$recorded_digest" =~ ^[0-9a-f]{64}$ ]] || reject 'caller manifest digest differs from strict receipt'
  mode='dispatch'
fi

if [[ "$mode" == receipt ]]; then
  printf 'task13_assertion_generation=%s\n' "$generation_id"
  printf 'task13_assertion_manifest_sha256=%s\n' "$manifest_sha"
  printf '%s\n' 'DISPOSITION: ACTIVE_R25_MANIFEST_VERIFIED (zero mutation)'
  exit 0
fi

pair="${target_justfile}::${target_recipe}"
case "$pair" in
  "$shape_relative::aarch64-linux-shape-r25-immutable"|scripts/tasks/task-1/mod.just::flake-linux|scripts/tasks/task-1/mod.just::flake-local|scripts/tasks/task-1/mod.just::dependency-check|scripts/tasks/task-1/mod.just::platform-check|scripts/tasks/task-1/mod.just::secret-scan|scripts/tasks/task-11/mod.just::push-barriers-green|scripts/tasks/task-12/mod.just::composition-green|scripts/tasks/task-12/mod.just::contract-green|scripts/tasks/task-12/mod.just::migration-chain-green|scripts/tasks/task-3b/mod.just::check|scripts/tasks/task-4a/mod.just::postgres-recovery|scripts/tasks/task-4b/mod.just::redis-publish-config-green|scripts/tasks/task-4b/mod.just::redis-recovery|scripts/tasks/task-8/mod.just::contract-green|scripts/tasks/task-8/mod.just::resilience-green|scripts/tasks/task-9/mod.just::delivery-lifecycle-green|scripts/tasks/task-9/mod.just::privacy-mutations-green|"$shape_relative::coverage-r25-exec"|"$shape_relative::module-eval-r25-exec"|"$shape_relative::linux-builder-r25-exec"|"$shape_relative::aarch64-linux-builder-r25-exec"|"$shape_relative::cross-system-verify-r25-exec"|"$shape_relative::final-verify-r25-exec") ;;
  *) reject 'Justfile and recipe pair is not in exact R25 allowlist' ;;
esac
case "$target_recipe" in
  module-eval-r25-exec) ((${#recipe_arguments[@]} == 1)) && [[ "${recipe_arguments[0]}" == s2-skeleton || "${recipe_arguments[0]}" == s4-green || "${recipe_arguments[0]}" == s5-final ]] || reject 'module evaluation requires exactly one allowlisted phase' ;;
  linux-builder-r25-exec|aarch64-linux-builder-r25-exec|cross-system-verify-r25-exec)
    ((${#recipe_arguments[@]} == 1 || ${#recipe_arguments[@]} == 2)) && [[ "${recipe_arguments[0]}" =~ ^[0-9a-f]{64}$ ]] || reject 'descriptor requires lowercase GPE digest'
    ((${#recipe_arguments[@]} == 1)) || [[ "${recipe_arguments[1]}" == fv07-replay ]] || reject 'descriptor replay must be literal fv07-replay'
    ;;
  final-verify-r25-exec)
    ((${#recipe_arguments[@]} == 2)) || reject 'final verification requires selector and GPE digest'
    case "${recipe_arguments[0]}" in fv01|fv02|fv03|fv04|fv05|fv06|fv07|fv08) ;; *) reject 'final verification selector is not allowlisted' ;; esac
    [[ "${recipe_arguments[1]}" =~ ^[0-9a-f]{64}$ ]] || reject 'final verification GPE digest must be lowercase 64-hex'
    ;;
  *) ((${#recipe_arguments[@]} == 0)) || reject 'selected pair accepts zero recipe arguments' ;;
esac

execution_class="$(jq -er --arg pair "$pair" '[.guarded_dispatch.execution_classes | to_entries[] | select(any(.value[]; . == $pair)) | .key] | if length == 1 then .[0] else error("class") end' "$manifest_copy")" || reject 'pair execution class is invalid'
verify_repo "$target_justfile" 'selected Justfile'
source_path="$repo_root/$target_justfile"
source_identity="$(identity "$source_path")"
if [[ "$target_justfile" == "$shape_relative" ]]; then
  source_sha="$(manifest_source_sha "$shape_relative")" || reject 'manifest shape hash is missing'
else
  source_sha="$(sha256_file "$source_path")"
fi

snapshot=''
if [[ "$execution_class" == immutable_snapshot ]]; then
  copy_helper "$snapshot_helper_relative" snapshot-helper
  snapshot_helper_copy="$COPIED_PATH"; snapshot_helper_sha="$COPIED_SHA"
  snapshot="$(TASK13_SOURCE_ROOT="$repo_root" TASK13_R25_REPO_ROOT="$repo_root" TASK13_R25_MANIFEST_COPY="$manifest_copy" TASK13_R25_SNAPSHOT_HELPER_SHA256="$snapshot_helper_sha" bash "$snapshot_helper_copy")" || reject 'immutable source snapshot creation failed'
  [[ "$snapshot" == /nix/store/* && -d "$snapshot" && "$snapshot" != *$'\n'* ]] || reject 'immutable source snapshot is invalid'
  [[ -f "$snapshot/$target_justfile" && ! -L "$snapshot/$target_justfile" ]] || reject 'snapshot Justfile is missing or linked'
  if [[ "$target_justfile" == "$shape_relative" ]]; then
    [[ "$(sha256_file "$snapshot/$target_justfile")" == "$source_sha" ]] || reject 'immutable shape differs from manifest source hash'
    # The snapshot is the verified selected source for this immutable shape
    # pair; do not treat a mutable live path as its final source receipt.
    source_path="$snapshot/$target_justfile"
    source_identity="$(identity "$source_path")"
    copy_external "$source_path" "$source_sha" selected-justfile
    selected_copy="$COPIED_PATH"
    executable_path="$selected_copy"
  else
    executable_path="$snapshot/$target_justfile"
    source_path="$snapshot/$target_justfile"
    source_identity="$(identity "$source_path")"
    source_sha="$(sha256_file "$source_path")"
  fi
else
  [[ "$execution_class" == live_worktree ]] || reject 'pair execution class is invalid'
  if [[ "$target_justfile" == "$shape_relative" ]]; then
    [[ "$(sha256_file "$source_path")" == "$source_sha" ]] || reject 'live shape differs from manifest hash'
    copy_external "$source_path" "$source_sha" selected-justfile
  else
    copy_adjacent "$source_path" "$source_sha"
  fi
  selected_copy="$COPIED_PATH"
  executable_path="$selected_copy"
fi

# Exact helper-copy matrix: snapshot only for immutable; final-tree only for
# module/descriptors; final-tree plus final-verify only for final verification.
case "$target_recipe" in
  module-eval-r25-exec|linux-builder-r25-exec|aarch64-linux-builder-r25-exec|cross-system-verify-r25-exec|final-verify-r25-exec)
    copy_helper "$final_tree_helper_relative" final-tree-helper
    final_tree_helper_copy="$COPIED_PATH"; final_tree_helper_sha="$COPIED_SHA"
    ;;
esac
if [[ "$target_recipe" == final-verify-r25-exec ]]; then
  copy_helper "$final_verify_helper_relative" final-verify-helper
  final_verify_helper_copy="$COPIED_PATH"; final_verify_helper_sha="$COPIED_SHA"
fi

# Last possible mutation barrier, restricted to the isolated R25 fixture.
fixture_mode="${TASK13_R25_FIXTURE_MODE:-}"
if [[ -n "$fixture_mode" ]]; then
  [[ "$pair" == "$shape_relative::coverage-r25-exec" && "${TASK13_R25_FIXTURE_REPO_ROOT:-}" == "$repo_root" ]] || reject 'R25 fixture mode is restricted to disposable coverage'
  case "$fixture_mode" in source-postcopy-content|source-postcopy-replacement|copy-content|copy-replacement) ;; *) reject 'R25 fixture mode is invalid' ;; esac
  [[ "${TASK13_R25_FIXTURE_READY_FD:-}" =~ ^[0-9]+$ && "${TASK13_R25_FIXTURE_RELEASE_FD:-}" =~ ^[0-9]+$ ]] || reject 'R25 fixture descriptors are invalid'
  # The fixture receives only disposable paths.  It needs both identities to
  # exercise source and selected-copy mutation independently.
  printf 'ready:%s:%s:%s\n' "$fixture_mode" "$source_path" "$executable_path" >&"${TASK13_R25_FIXTURE_READY_FD}" || reject 'R25 fixture ready signal failed'
  IFS= read -r release <&"${TASK13_R25_FIXTURE_RELEASE_FD}" || reject 'R25 fixture release signal failed'
  [[ "$release" == "release:${fixture_mode}" ]] || reject 'R25 fixture release token is invalid'
fi

verify_external "$manifest_copy" "$manifest_sha" 'final-manifest'
verify_external "$strict_copy" "$strict_sha" 'final-strict'
verify_external "$guard_invocation" "$guard_sha" 'final-guard'
[[ "$(identity "$manifest_copy")" == "$manifest_identity" && "$(identity "$strict_copy")" == "$strict_identity" && "$(identity "$guard_invocation")" == "$guard_identity" ]] || reject 'bootstrap receipt identity changed before nested dispatch'
[[ -f "$source_path" && ! -L "$source_path" && "$(identity "$source_path")" == "$source_identity" && "$(sha256_file "$source_path")" == "$source_sha" ]] || reject 'bound Justfile source changed before nested dispatch'
if [[ "$executable_path" != "$source_path" ]]; then
  [[ -f "$executable_path" && ! -L "$executable_path" && "$(stat -c '%a' -- "$executable_path")" == 400 && "$(sha256_file "$executable_path")" == "$source_sha" ]] || reject 'selected Justfile copy changed before nested dispatch'
fi
assert_no_just_imports "$source_path" "$executable_path" 'final-pre-dispatch'
[[ -z "$final_tree_helper_copy" || "$(sha256_file "$final_tree_helper_copy")" == "$final_tree_helper_sha" ]] || reject 'final-tree helper copy changed'
[[ -z "$final_verify_helper_copy" || "$(sha256_file "$final_verify_helper_copy")" == "$final_verify_helper_sha" ]] || reject 'final-verify helper copy changed'

export TASK13_ASSERTION_GENERATION="$generation_id"
export TASK13_ASSERTION_DIGEST="$manifest_sha"
export TASK13_R25_ASSERTION_GENERATION="$generation_id"
export TASK13_R25_ASSERTION_DIGEST="$manifest_sha"
export TASK13_R25_SELECTED_JUSTFILE_COPY="$executable_path"
export TASK13_R25_SELECTED_JUSTFILE_SHA256="$source_sha"
export TASK13_R25_SELECTED_JUSTFILE_IDENTITY="$(identity "$executable_path")"
export TASK13_BOUND_JUSTFILE="$TASK13_R25_SELECTED_JUSTFILE_COPY"
export TASK13_BOUND_JUSTFILE_SHA256="$TASK13_R25_SELECTED_JUSTFILE_SHA256"
export TASK13_BOUND_JUSTFILE_IDENTITY="$TASK13_R25_SELECTED_JUSTFILE_IDENTITY"
[[ -z "$snapshot" ]] || export TASK13_FLAKE_SOURCE_SNAPSHOT="$snapshot"
[[ -z "$snapshot_helper_copy" ]] || { export TASK13_R25_SNAPSHOT_HELPER_COPY="$snapshot_helper_copy"; export TASK13_R25_SNAPSHOT_HELPER_SHA256="$snapshot_helper_sha"; }
[[ -z "$final_tree_helper_copy" ]] || { export TASK13_R25_FINAL_TREE_HELPER_COPY="$final_tree_helper_copy"; export TASK13_R25_FINAL_TREE_HELPER_SHA256="$final_tree_helper_sha"; }
[[ -z "$final_verify_helper_copy" ]] || { export TASK13_R25_FINAL_VERIFY_HELPER_COPY="$final_verify_helper_copy"; export TASK13_R25_FINAL_VERIFY_HELPER_SHA256="$final_verify_helper_sha"; }
unset TASK13_R25_GPE_MAP_SHA256 TASK13_R25_REPLAY_MODE TASK13_R25_FV_SELECTOR
case "$target_recipe" in
  linux-builder-r25-exec|aarch64-linux-builder-r25-exec|cross-system-verify-r25-exec)
    export TASK13_R25_GPE_MAP_SHA256="${recipe_arguments[0]}"; ((${#recipe_arguments[@]} == 1)) || export TASK13_R25_REPLAY_MODE=fv07-replay ;;
  final-verify-r25-exec)
    export TASK13_R25_FV_SELECTOR="${recipe_arguments[0]}"; export TASK13_R25_GPE_MAP_SHA256="${recipe_arguments[1]}" ;;
esac
printf 'task13_assertion_manifest_sha256=%s\n' "$manifest_sha"
printf 'task13_assertion_generation=%s\n' "$generation_id"
set +e
if [[ "$target_justfile" == "$shape_relative" ]]; then
  work_root="${snapshot:-$repo_root}"
  just --working-directory "$work_root" --justfile "$executable_path" "$target_recipe" "${recipe_arguments[@]}"
else
  (cd "${snapshot:-$repo_root}" && just --justfile "$executable_path" "$target_recipe" "${recipe_arguments[@]}")
fi
status=$?
set -e
exit "$status"
