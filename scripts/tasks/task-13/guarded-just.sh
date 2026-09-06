#!/usr/bin/env bash
set -euo pipefail

readonly historical_generation='task13-r11-historical-20260902'
readonly active_generation='task13-r12-aarch64-linux-20260905'
if (($# == 3)); then
  generation_id="$historical_generation"
elif (($# == 4)); then
  generation_id="$1"
  shift
  if [[ "$generation_id" != "$active_generation" ]]; then
    printf 'error: unknown Task-13 assertion generation %s\n' "$generation_id" >&2
    exit 2
  fi
else
  printf '%s\n' 'usage: guarded-just.sh [task13-r12-aarch64-linux-20260905] <recorded-digest> <justfile-relative-path> <recipe>' >&2
  exit 2
fi

readonly generation_id
readonly recorded_digest="$1"
readonly target_justfile="$2"
readonly target_recipe="$3"
if [[ "$generation_id" == "$active_generation" ]]; then
  readonly manifest='tests/nix/task-13-assertion-manifest-aarch64-linux.json'
  readonly lock='tests/nix/task-13-assertion-manifest-aarch64-linux.sha256'
  readonly fixed_record='.agents/results/task-13-s2-aarch64-linux-manifest-digest-20260905.txt'
else
  readonly manifest='tests/nix/task-13-assertion-manifest.json'
  readonly lock='tests/nix/task-13-assertion-manifest.sha256'
  readonly fixed_record='.agents/results/task-13-s1-manifest-digest-20260902-090159.txt'
fi
readonly historical_digest='d3aaca6c53e8e4fa5f5dd6077e1bad92166529579a75a2bb67625faa056c49bb'
readonly repo_root="$(pwd -P)"
manifest_snapshot=''
snapshot_helper_copy=''

cleanup() {
  [[ -z "$manifest_snapshot" ]] || rm -f -- "$manifest_snapshot"
  [[ -z "$snapshot_helper_copy" ]] || rm -f -- "$snapshot_helper_copy"
}
trap cleanup EXIT

reject() {
  printf 'error: %s\n' "$1" >&2
  exit 2
}

[[ "$recorded_digest" =~ ^[0-9a-f]{64}$ ]] || reject 'recorded digest must be lowercase 64-hex'
if [[ "$generation_id" == "$active_generation" ]]; then
  [[ "$target_justfile" == 'scripts/tasks/task-13/r12-shape.just' || "$target_justfile" =~ ^scripts/tasks/[A-Za-z0-9-]+/mod\.just$ ]] || reject 'justfile path is not canonical for active R12'
else
  [[ "$target_justfile" =~ ^scripts/tasks/[A-Za-z0-9-]+/mod\.just$ ]] || reject 'justfile path is not canonical for historical R11'
fi
[[ "$target_recipe" =~ ^[A-Za-z0-9][A-Za-z0-9-]*$ ]] || reject 'recipe is not canonical'
[[ -f "$manifest" && -f "$lock" && -f "$fixed_record" ]] || reject 'manifest, lock, or fixed coordinator digest record is missing'
[[ ! -L "$target_justfile" ]] || reject 'justfile final path must not be a symlink'
if [[ "$generation_id" == "$active_generation" ]]; then
  [[ "$recorded_digest" != "$historical_digest" ]] || reject 'historical R11 digest is not valid for active R12 cards'
  cmp -s <(jq -cS '.' "$manifest") "$manifest" || reject 'active R12 manifest bytes are not canonical JSON plus LF'
  jq -e --arg generation_id "$active_generation" '.generation_id == $generation_id and .flake_shape.active_generation == $generation_id' "$manifest" >/dev/null || reject 'active R12 generation identity is missing or mismatched'
fi

manifest_snapshot="$(mktemp "${TMPDIR:-/tmp}/jamye-task13-manifest.XXXXXX")" || reject 'cannot create manifest snapshot'
cp -- "$manifest" "$manifest_snapshot" || reject 'cannot snapshot manifest'
snapshot_digest="$(sha256sum "$manifest_snapshot" | awk '{print $1}')"
[[ "$snapshot_digest" =~ ^[0-9a-f]{64}$ ]] || reject 'manifest snapshot digest is malformed'

verify_inputs_against_snapshot() {
  local expected_lock expected_record
  expected_lock="${snapshot_digest}"$'\n'
  expected_record="${snapshot_digest}"$'\n'
  cmp -s <(printf '%s' "$expected_lock") "$lock" || reject 'manifest lock is malformed or mismatched'
  cmp -s <(printf '%s' "$expected_record") "$fixed_record" || reject 'fixed coordinator record is malformed or mismatched'
  [[ "$recorded_digest" == "$snapshot_digest" ]] || reject 'Task-13 manifest digest mismatch'
  cmp -s "$manifest_snapshot" "$manifest" || reject 'live manifest changed from verified snapshot'
  [[ "$(sha256sum "$manifest" | awk '{print $1}')" == "$snapshot_digest" ]] || reject 'live manifest digest changed from verified snapshot'
}

verify_active_g0_prerequisite() {
  local token='.agents/results/task-4b-redis-publish-completion-20260902-090159.json'
  local expected_token_sha='e91b29a683fad3ba5bc1253a381b9689b932a0d08b2d43dcaf65d25e84f2814c'
  local expected_sha target_file evidence_file

  [[ -f "$token" && ! -L "$token" ]] || reject 'missing or linked Task-4b completion token'
  [[ "$(sha256sum "$token" | awk '{print $1}')" == "$expected_token_sha" ]] || reject 'stale Task-4b completion token'
  jq -e '.status == "complete" and .no_scope_expansion == true and .freshness.allowed_file_hashes_match_at_completion == true and .freshness.evidence_hashes_match_at_completion == true and .freshness.review_hash_matches_at_completion == true and .scaffold_review.status == "PASS" and .fresh_review.status == "PASS"' "$token" >/dev/null || reject 'Task-4b completion token contract is invalid'
  while IFS=$'\t' read -r expected_sha target_file; do
    [[ "$(sha256sum "$target_file" | awk '{print $1}')" == "$expected_sha" ]] || reject 'stale G0 allowed input'
  done < <(jq -r '.allowed_file_sha256 | to_entries[] | "\(.value)\t\(.key)"' "$token")
  while IFS=$'\t' read -r expected_sha evidence_file; do
    [[ "$(sha256sum "$evidence_file" | awk '{print $1}')" == "$expected_sha" ]] || reject 'stale G0 evidence'
  done < <(jq -r '.red, .green, .scaffold_review, .fresh_review | "\(.sha256 // .evidence_sha256)\t\(.path // .evidence_path)"' "$token")
  printf '%s\n' 'Task-4b Redis publish completion token is current'
}

verify_active_command_source_schema() {
  jq -e --arg generation_id "$active_generation" '
    .command_source_integrity.generation_id == $generation_id
    and (.command_source_integrity.canonical_order == [
      "scripts/tasks/task-13/r12-shape.just",
      "scripts/tasks/task-13/guarded-just.sh",
      "tests/nix/flake-shape-aarch64-linux.sh",
      "scripts/tasks/task-13/flake-source-snapshot.sh"
    ])
    and ([.command_source_integrity.sources[].path] == .command_source_integrity.canonical_order)
    and (all(.command_source_integrity.sources[];
      (.path | test("^(scripts/tasks/task-13/(r12-shape[.]just|guarded-just[.]sh|flake-source-snapshot[.]sh)|tests/nix/flake-shape-aarch64-linux[.]sh)$"))
      and (.sha256 | test("^[0-9a-f]{64}$"))
    ))
    and (.command_source_integrity.post_validation_mutation_fixture.id == "active-r12-post-validation-live-justfile-mutation-zero-nested-dispatch")
    and (.command_source_integrity.post_validation_mutation_fixture.target == "scripts/tasks/task-13/r12-shape.just")
    and (.command_source_integrity.post_validation_mutation_fixture.mutations == ["content-mutation", "path-replacement"])
    and (.command_source_integrity.post_validation_mutation_fixture.expected_nested_dispatch_count == 0)
    and (.command_source_integrity.post_validation_mutation_fixture.expected_exit == 2)
    and (.command_source_integrity.reused_target_sources | length == 1)
    and (.command_source_integrity.reused_target_sources[0].path == "scripts/tasks/task-1/mod.just")
    and (.command_source_integrity.reused_target_sources[0].applies_to_recipes == ["flake-local", "flake-linux"])
    and (.command_source_integrity.reused_target_sources[0].sha256 | test("^[0-9a-f]{64}$"))
    and (.guarded_dispatch.execution_classes.immutable_snapshot == [
      {"justfile":"scripts/tasks/task-13/r12-shape.just","recipe":"aarch64-linux-shape-immutable"},
      {"justfile":"scripts/tasks/task-1/mod.just","recipe":"flake-linux"},
      {"justfile":"scripts/tasks/task-1/mod.just","recipe":"flake-local"}
    ])
    and ((.guarded_dispatch.execution_classes.immutable_snapshot + .guarded_dispatch.execution_classes.live_worktree) as $classified
      | ($classified | length) == (.guarded_dispatch.allowed_pairs | length)
      and ([$classified[] | "\(.justfile)\u0000\(.recipe)"] | sort) == ([.guarded_dispatch.allowed_pairs[] | "\(.justfile)\u0000\(.recipe)"] | sort)
      and ([$classified[] | "\(.justfile)\u0000\(.recipe)"] | unique | length) == ($classified | length)
    )
  ' "$manifest_snapshot" >/dev/null || reject 'active R12 command-source integrity schema is missing or incompatible'
}

active_execution_class_for_pair() {
  jq -er --arg justfile "$target_justfile" --arg recipe "$target_recipe" '
    [.guarded_dispatch.execution_classes
      | to_entries[]
      | select(any(.value[]; .justfile == $justfile and .recipe == $recipe))
      | .key]
    | if length == 1 then .[0] else error("active pair classification must be exact") end
  ' "$manifest_snapshot"
}

verify_recorded_source_at_root() {
  local source_root="$1"
  local phase="$2"
  local expected_sha="$3"
  local relative_path="$4"
  local source_directory source_basename physical_directory physical_source actual_sha

  [[ "$source_root" == /* && -d "$source_root" ]] || reject "active R12 command-source root is invalid at ${phase}"
  [[ "$expected_sha" =~ ^[0-9a-f]{64}$ ]] || reject "active R12 command-source digest is malformed at ${phase}"
  case "$relative_path" in
    scripts/tasks/task-13/r12-shape.just | scripts/tasks/task-13/guarded-just.sh | tests/nix/flake-shape-aarch64-linux.sh | scripts/tasks/task-13/flake-source-snapshot.sh | scripts/tasks/task-1/mod.just) ;;
    *) reject "active R12 command-source path is noncanonical at ${phase}" ;;
  esac
  source_directory="$source_root/$(dirname -- "$relative_path")"
  source_basename="$(basename -- "$relative_path")"
  [[ -d "$source_directory" ]] || reject "active R12 command-source directory is missing at ${phase}"
  physical_directory="$(cd -P -- "$source_directory" && pwd -P)" || reject "active R12 command-source directory cannot be resolved at ${phase}"
  physical_source="${physical_directory}/${source_basename}"
  [[ "$physical_source" == "$source_root/$relative_path" && -f "$physical_source" && ! -L "$physical_source" ]] || reject "active R12 command source is missing, linked, or escaped at ${phase}"
  actual_sha="$(sha256sum "$physical_source" | awk '{print $1}')"
  [[ "$actual_sha" == "$expected_sha" ]] || reject "active R12 command-source hash mismatch at ${phase}"
}

verify_command_sources_at_root() {
  local source_root="$1"
  local phase="$2"
  local expected_sha relative_path

  while IFS=$'\t' read -r expected_sha relative_path; do
    verify_recorded_source_at_root "$source_root" "$phase" "$expected_sha" "$relative_path"
  done < <(jq -r '.command_source_integrity.sources[] | "\(.sha256)\t\(.path)"' "$manifest_snapshot")
}

verify_reused_task1_source_at_root() {
  local source_root="$1"
  local phase="$2"
  local expected_sha relative_path

  expected_sha="$(jq -er '.command_source_integrity.reused_target_sources[0].sha256' "$manifest_snapshot")" || reject 'active R12 reused Task-1 source digest is missing'
  relative_path="$(jq -er '.command_source_integrity.reused_target_sources[0].path' "$manifest_snapshot")" || reject 'active R12 reused Task-1 source path is missing'
  verify_recorded_source_at_root "$source_root" "$phase" "$expected_sha" "$relative_path"
}

allowed_pairs_from_snapshot() {
  if [[ "$generation_id" == "$active_generation" ]]; then
    jq -r '
      .guarded_dispatch.allowed_pairs[]
      | select(
          ((.justfile == "scripts/tasks/task-13/r12-shape.just") or (.justfile | test("^scripts/tasks/[A-Za-z0-9-]+/mod\\.just$")))
          and (.recipe | test("^[A-Za-z0-9][A-Za-z0-9-]*$"))
        )
      | "\(.justfile)\t\(.recipe)"
    ' "$manifest_snapshot" | LC_ALL=C sort -u
  else
    {
      printf '%s\n' $'scripts/tasks/task-13/mod.just\tred'
      printf '%s\n' $'scripts/tasks/task-1/mod.just\tflake-local'
      printf '%s\n' $'scripts/tasks/task-1/mod.just\tflake-linux'
      jq -r '
        .. | strings
        | select(test("^scripts/tasks/task-13/guarded-just\\.sh <recorded-digest> scripts/tasks/[A-Za-z0-9-]+/mod\\.just [A-Za-z0-9][A-Za-z0-9-]*$"))
        | split(" ")
        | "\(.[2])\t\(.[3])"
      ' "$manifest_snapshot"
    } | LC_ALL=C sort -u
  fi
}

verify_inputs_against_snapshot
if ! allowed_pairs_from_snapshot | grep -Fqx "${target_justfile}"$'\t'"${target_recipe}"; then
  reject 'justfile and recipe pair is not allowlisted by the verified manifest snapshot'
fi

target_directory="$(dirname -- "$target_justfile")"
target_basename="$(basename -- "$target_justfile")"
physical_directory="$(cd -P -- "$target_directory" && pwd -P)" || reject 'justfile path cannot be resolved'
physical_justfile="${physical_directory}/${target_basename}"
[[ "$physical_justfile" == "$repo_root/$target_justfile" && -f "$physical_justfile" ]] || reject 'justfile path escapes or changes the approved task identity'

# Revalidate byte identity immediately before dispatch. If A→B→A occurs, the
# B allowlist cannot survive because it was derived only from this A snapshot.
verify_inputs_against_snapshot
[[ ! -L "$target_justfile" ]] || reject 'justfile final path became a symlink before dispatch'
dispatch_directory="$(cd -P -- "$target_directory" && pwd -P)" || reject 'justfile path cannot be resolved before dispatch'
dispatch_justfile="${dispatch_directory}/${target_basename}"
[[ "$dispatch_justfile" == "$repo_root/$target_justfile" && -f "$dispatch_justfile" ]] || reject 'justfile path changed approved task identity before dispatch'

if [[ "$generation_id" == "$active_generation" ]]; then
  verify_active_command_source_schema
  verify_active_g0_prerequisite
  active_execution_class="$(active_execution_class_for_pair)" || reject 'active R12 dispatch pair classification failed'
  verify_command_sources_at_root "$repo_root" 'live-before-dispatch'

  if [[ "$active_execution_class" == 'immutable_snapshot' ]]; then
    if [[ "$target_justfile" == 'scripts/tasks/task-1/mod.just' ]]; then
      verify_reused_task1_source_at_root "$repo_root" 'live-before-snapshot'
    fi

    snapshot_helper_relative='scripts/tasks/task-13/flake-source-snapshot.sh'
    snapshot_helper_expected_sha="$(jq -er --arg path "$snapshot_helper_relative" '.command_source_integrity.sources[] | select(.path == $path) | .sha256' "$manifest_snapshot")" || reject 'active R12 snapshot-helper hash is missing'
    snapshot_helper_copy="$(mktemp "${TMPDIR:-/tmp}/jamye-task13-snapshot-helper.XXXXXX")" || reject 'cannot create verified snapshot-helper copy'
    cp -- "$repo_root/$snapshot_helper_relative" "$snapshot_helper_copy" || reject 'cannot copy the verified snapshot helper'
    [[ "$(sha256sum "$snapshot_helper_copy" | awk '{print $1}')" == "$snapshot_helper_expected_sha" ]] || reject 'verified snapshot-helper copy hash mismatch'

    if ! source_snapshot="$(TASK13_SOURCE_ROOT="$repo_root" bash "$snapshot_helper_copy")"; then
      reject 'filtered source snapshot creation failed'
    fi
    [[ "$source_snapshot" == /nix/store/* && "$source_snapshot" != *$'\n'* && -d "$source_snapshot" ]] || reject 'filtered source snapshot is not a Nix store directory'

    verify_inputs_against_snapshot
    snapshot_manifest="$source_snapshot/$manifest"
    [[ -f "$snapshot_manifest" && ! -L "$snapshot_manifest" ]] || reject 'filtered source snapshot active manifest is missing or linked'
    cmp -s "$manifest_snapshot" "$snapshot_manifest" || reject 'filtered source snapshot active manifest differs from the verified manifest'
    [[ "$(sha256sum "$snapshot_manifest" | awk '{print $1}')" == "$snapshot_digest" ]] || reject 'filtered source snapshot active manifest digest mismatch'
    verify_command_sources_at_root "$source_snapshot" 'immutable-snapshot'
    if [[ "$target_justfile" == 'scripts/tasks/task-1/mod.just' ]]; then
      verify_reused_task1_source_at_root "$source_snapshot" 'immutable-snapshot'
    fi

    snapshot_target_directory="$(cd -P -- "$source_snapshot/$target_directory" && pwd -P)" || reject 'filtered source snapshot justfile directory cannot be resolved'
    snapshot_justfile="${snapshot_target_directory}/${target_basename}"
    [[ "$snapshot_justfile" == "$source_snapshot/$target_justfile" && -f "$snapshot_justfile" && ! -L "$snapshot_justfile" ]] || reject 'filtered source snapshot justfile is invalid'

    [[ ! -L "$target_justfile" ]] || reject 'live justfile became a symlink after snapshot creation'
    final_dispatch_directory="$(cd -P -- "$target_directory" && pwd -P)" || reject 'live justfile path cannot be resolved after snapshot creation'
    final_dispatch_justfile="${final_dispatch_directory}/${target_basename}"
    [[ "$final_dispatch_justfile" == "$repo_root/$target_justfile" && -f "$final_dispatch_justfile" ]] || reject 'live justfile changed approved task identity after snapshot creation'
    cmp -s "$snapshot_justfile" "$final_dispatch_justfile" || reject 'filtered source snapshot justfile differs from validated live justfile'

    # Static fixture contract: a mutation or path replacement after the first
    # live validation is caught here, before the sole nested Just dispatch.
    verify_command_sources_at_root "$repo_root" 'live-after-snapshot'
    if [[ "$target_justfile" == 'scripts/tasks/task-1/mod.just' ]]; then
      verify_reused_task1_source_at_root "$repo_root" 'live-after-snapshot'
    fi
    verify_inputs_against_snapshot

    printf 'task13_assertion_generation=%s\n' "$generation_id"
    printf 'task13_assertion_manifest_sha256=%s\n' "$snapshot_digest"
    printf 'task13_flake_source_snapshot=%s\n' "$source_snapshot"
    rm -f -- "$manifest_snapshot" "$snapshot_helper_copy"
    manifest_snapshot=''
    snapshot_helper_copy=''
    cd "$source_snapshot"
    export TASK13_ASSERTION_GENERATION="$generation_id"
    export TASK13_ASSERTION_DIGEST="$snapshot_digest"
    export TASK13_FLAKE_SOURCE_SNAPSHOT="$source_snapshot"
    exec just --justfile "$snapshot_justfile" "$target_recipe"
  fi

  [[ "$active_execution_class" == 'live_worktree' ]] || reject 'active R12 dispatch pair has an invalid execution class'
  verify_command_sources_at_root "$repo_root" 'live-before-worktree-dispatch'
  verify_inputs_against_snapshot
fi

is_task1_flake_pair=false
if [[ "$target_justfile" == 'scripts/tasks/task-1/mod.just' && ( "$target_recipe" == 'flake-local' || "$target_recipe" == 'flake-linux' ) ]]; then
  is_task1_flake_pair=true
fi

if "$is_task1_flake_pair"; then
  if ! source_snapshot="$(bash scripts/tasks/task-13/flake-source-snapshot.sh)"; then
    reject 'filtered source snapshot creation failed'
  fi
  [[ "$source_snapshot" == /nix/store/* && -d "$source_snapshot" ]] || reject 'filtered source snapshot is not a Nix store directory'
  snapshot_justfile="$source_snapshot/$target_justfile"
  [[ "$snapshot_justfile" == "$source_snapshot"/scripts/tasks/task-1/mod.just && -f "$snapshot_justfile" && ! -L "$snapshot_justfile" ]] || reject 'filtered source snapshot Task-1 justfile is invalid'
  verify_inputs_against_snapshot
  [[ ! -L "$target_justfile" ]] || reject 'live Task-1 justfile became a symlink after snapshot creation'
  final_dispatch_directory="$(cd -P -- "$target_directory" && pwd -P)" || reject 'live Task-1 justfile path cannot be resolved after snapshot creation'
  final_dispatch_justfile="${final_dispatch_directory}/${target_basename}"
  [[ "$final_dispatch_justfile" == "$repo_root/$target_justfile" && -f "$final_dispatch_justfile" ]] || reject 'live Task-1 justfile changed approved task identity after snapshot creation'
  cmp -s "$snapshot_justfile" "$final_dispatch_justfile" || reject 'filtered source snapshot Task-1 justfile differs from validated live justfile'
  if [[ "$generation_id" == "$active_generation" ]]; then
    printf 'task13_assertion_generation=%s\n' "$generation_id"
  fi
  printf 'task13_assertion_manifest_sha256=%s\n' "$snapshot_digest"
  printf 'task13_flake_source_snapshot=%s\n' "$source_snapshot"
  rm -f -- "$manifest_snapshot"
  manifest_snapshot=''
  cd "$source_snapshot"
  export TASK13_ASSERTION_GENERATION="$generation_id"
  export TASK13_ASSERTION_DIGEST="$snapshot_digest"
  exec just --justfile "$snapshot_justfile" "$target_recipe"
fi

if [[ "$generation_id" == "$active_generation" ]]; then
  printf 'task13_assertion_generation=%s\n' "$generation_id"
fi
printf 'task13_assertion_manifest_sha256=%s\n' "$snapshot_digest"
# exec does not run Bash EXIT traps on success; remove the verified snapshot now.
rm -f -- "$manifest_snapshot"
manifest_snapshot=''
export TASK13_ASSERTION_GENERATION="$generation_id"
export TASK13_ASSERTION_DIGEST="$snapshot_digest"
exec just --justfile "$target_justfile" "$target_recipe"
