#!/usr/bin/env bash
set -euo pipefail

readonly active_generation='task13-r13-g0-f1-format-drift-20260905'
readonly manifest='tests/nix/task-13-assertion-manifest-r13-g0-f1.json'
readonly lock='tests/nix/task-13-assertion-manifest-r13-g0-f1.sha256'
readonly fixed_record='.agents/results/task-13-s2-r13-g0-f1-manifest-digest-20260905.txt'
readonly g0_token='.agents/results/task-4b-redis-publish-completion-g0-f1-20260905.json'
readonly expected_g0_token_sha='f178eb60c2c18c27feeea8e0da13a11700dd82372310c426085992547fdd95be'
readonly g0_token_review='.agents/results/review-task13-g0-f1-token-static-r2-20260905.md'
readonly expected_g0_token_review_sha='71b33c28ebd337c076065b4442311d977845fd8e0f1f9d225b417c6f526d9e62'
readonly script_directory="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly repo_root="$(cd -P -- "$script_directory/../../.." && pwd -P)"

mode=''
if (($# == 2)) && [[ "$2" == '--receipt' ]]; then
  mode='receipt'
  generation_id="$1"
elif (($# == 4)); then
  mode='dispatch'
  generation_id="$1"
  recorded_digest="$2"
  target_justfile="$3"
  target_recipe="$4"
else
  printf '%s\n' 'usage: guarded-just-r13.sh task13-r13-g0-f1-format-drift-20260905 (--receipt | <recorded-digest> <justfile-relative-path> <recipe>)' >&2
  exit 2
fi

readonly mode generation_id
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

[[ "$(pwd -P)" == "$repo_root" ]] || reject 'R13 guard must start at the repository root'
[[ "$generation_id" == "$active_generation" ]] || reject 'unknown Task-13 R13 assertion generation'

verify_regular_repo_file() {
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

for required_path in "$manifest" "$lock" "$fixed_record"; do
  verify_regular_repo_file "$required_path" 'R13 manifest material'
done

verify_manifest_file() {
  local source_file="$1"
  local expected

  cmp -s <(jq -cS '.' "$source_file") "$source_file" || reject 'R13 manifest bytes are not canonical JSON plus LF'
  snapshot_digest="$(sha256sum "$source_file" | awk '{print $1}')"
  [[ "$snapshot_digest" =~ ^[0-9a-f]{64}$ ]] || reject 'R13 manifest digest is malformed'
  expected="${snapshot_digest}"$'\n'
  cmp -s <(printf '%s' "$expected") "$lock" || reject 'R13 manifest lock is malformed or mismatched'
  cmp -s <(printf '%s' "$expected") "$fixed_record" || reject 'R13 coordinator record is malformed or mismatched'
  jq -e --arg generation_id "$active_generation" '
    .generation_id == $generation_id
    and .flake_shape.active_generation == $generation_id
    and .flake_shape.green_only == true
    and (.flake_shape | has("baseline_red_failures") | not)
    and (.flake_shape | has("missing_system_red") | not)
  ' "$source_file" >/dev/null || reject 'R13 generation or GREEN-only identity is invalid'
}

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
  local expected_sha target_file evidence_file parent_token selector

  verify_regular_repo_file "$g0_token" 'G0-F1 completion token'
  verify_regular_repo_file "$g0_token_review" 'G0-F1 token review'
  [[ "$(sha256sum "$g0_token" | awk '{print $1}')" == "$expected_g0_token_sha" ]] || reject 'stale G0-F1 completion token'
  [[ "$(sha256sum "$g0_token_review" | awk '{print $1}')" == "$expected_g0_token_review_sha" ]] || reject 'stale G0-F1 token review'
  jq -e '
    .schema_version == "1.1"
    and .generation_id == "g0-f1-format-drift-20260905"
    and .status == "complete"
    and .prerequisite_id == "task-4b-redis-publish-config-binding"
    and .owner == "task-13/backend"
    and .parent_token.path == ".agents/results/task-4b-redis-publish-completion-20260902-090159.json"
    and .parent_token.sha256 == "e91b29a683fad3ba5bc1253a381b9689b932a0d08b2d43dcaf65d25e84f2814c"
    and .parent_token.status == "complete"
    and .supersedes.parent_path == .parent_token.path
    and .supersedes.parent_sha256 == .parent_token.sha256
    and .supersedes.active_replacement_field == "allowed_file_sha256"
    and .supersedes.parent_remains_immutable_historical_evidence == true
    and .supersedes.behavioral_contract_superseded == false
    and .supersedes.test_name_oracle_superseded == false
    and .supersedes.cargo_test_selector_superseded == false
    and .supersedes.historical_evidence_superseded == false
    and .supersedes.r11_or_r12_artifacts_superseded == false
    and (.authority.approvals | length == 3)
    and (.allowed_file_sha256 | keys == [
      "docs/commands/task-4b/realtime.md",
      "scripts/tasks/task-4b/mod.just",
      "src/config/mod.rs",
      "src/config/realtime.rs",
      "src/transport/realtime/composition.rs"
    ])
    and .format_only_delta.changed_count == 2
    and ([.format_only_delta.changed_files[].path] == [
      "src/config/realtime.rs",
      "src/transport/realtime/composition.rs"
    ])
    and all(.format_only_delta.changed_files[]; .classification == "rustfmt layout only")
    and .format_only_delta.unchanged_count == 3
    and ([.format_only_delta.unchanged_files[].path] == [
      "docs/commands/task-4b/realtime.md",
      "scripts/tasks/task-4b/mod.just",
      "src/config/mod.rs"
    ])
    and .format_only_delta.behavioral_contract_changed == false
    and .format_only_delta.test_name_oracle_changed == false
    and .format_only_delta.cargo_test_selector_changed == false
    and (.full_name_oracle_lexical == [
      "config::realtime::redis_publish_config_binding::defaults_are_exact_15000_2000_1000_ms",
      "config::realtime::redis_publish_config_binding::key_only_errors_never_echo_raw_values",
      "config::realtime::redis_publish_config_binding::partial_override_uses_remaining_defaults",
      "transport::realtime::composition::redis_publish_config_binding::all_three_overrides_materialize_outbox_worker_durations",
      "transport::realtime::composition::redis_publish_config_binding::arithmetic_overflow_fails_before_side_effects",
      "transport::realtime::composition::redis_publish_config_binding::equality_and_over_budget_fail_before_side_effects",
      "transport::realtime::composition::redis_publish_config_binding::existing_realtime_worker_default_regression",
      "transport::realtime::composition::redis_publish_config_binding::invalid_parse_and_bounds_fail_before_side_effects"
    ])
    and .oracle_unchanged_from_parent == true
    and .cargo_test_selector == "CARGO_NET_OFFLINE=true cargo test --locked --lib redis_publish_config_binding -- --nocapture"
    and .cargo_test_selector_unchanged_from_parent == true
    and .format_check.status == "PASS"
    and .format_check.exit_code == 0
    and .green.status == "PASS"
    and .green.discovered == 8
    and .green.passed == 8
    and .green.failed == 0
    and .green.ignored == 0
    and .green.filtered_out == 3
    and .green.exit_code == 0
    and .historical_red.retained_from_parent == true
    and .historical_red.discovered == 8
    and .historical_red.passed == 2
    and .historical_red.failed == 6
    and .historical_red.exit_code == 101
    and .fresh_review.status == "PASS"
    and .fresh_review.valid_findings == 0
    and .fresh_review.critical_findings == 0
    and .fresh_review.high_findings == 0
    and .fresh_review.medium_findings == 0
    and .fresh_review.low_findings == 0
    and all(.freshness[]; . == true or (type == "string"))
    and .no_scope_expansion == true
    and .scope_expansion_findings == []
    and .agent_executable_verification_performed == false
  ' "$g0_token" >/dev/null || reject 'G0-F1 completion token contract is invalid'

  parent_token="$(jq -er '.parent_token.path' "$g0_token")" || reject 'G0-F1 parent path missing'
  verify_regular_repo_file "$parent_token" 'G0-F1 parent token'
  jq -e --slurpfile parent "$parent_token" '
    . as $t
    | ($parent[0]) as $p
    | all(.format_only_delta.changed_files[];
        .parent_sha256 == $p.allowed_file_sha256[.path]
        and .current_sha256 == $t.allowed_file_sha256[.path])
      and all(.format_only_delta.unchanged_files[];
        .parent_sha256 == $p.allowed_file_sha256[.path]
        and .current_sha256 == $p.allowed_file_sha256[.path]
        and .current_sha256 == $t.allowed_file_sha256[.path])
      and $t.full_name_oracle_lexical == $p.full_name_oracle_lexical
      and $t.cargo_test_selector == $p.cargo_test_selector
  ' "$g0_token" >/dev/null || reject 'G0-F1 parent reconciliation is invalid'

  while IFS=$'\t' read -r expected_sha target_file; do
    verify_regular_repo_file "$target_file" 'G0-F1 allowed input'
    [[ "$(sha256sum "$target_file" | awk '{print $1}')" == "$expected_sha" ]] || reject "stale G0-F1 allowed input $target_file"
  done < <(jq -r '.allowed_file_sha256 | to_entries[] | "\(.value)\t\(.key)"' "$g0_token")

  while IFS=$'\t' read -r expected_sha evidence_file; do
    verify_regular_repo_file "$evidence_file" 'G0-F1 referenced evidence'
    [[ "$(sha256sum "$evidence_file" | awk '{print $1}')" == "$expected_sha" ]] || reject "stale G0-F1 evidence $evidence_file"
  done < <(jq -r '
    .authority.approved_plan,
    .authority.approved_requirements,
    .authority.approvals[],
    .parent_token,
    .execution_convention,
    .format_check,
    .historical_red,
    .green,
    .fresh_review
    | "\(.sha256 // .evidence_sha256)\t\(.path // .evidence_path)"
  ' "$g0_token")

  selector="$(jq -er '.cargo_test_selector' "$g0_token")" || reject 'G0-F1 selector missing'
  [[ "$(rg -F -c -- "$selector" scripts/tasks/task-4b/mod.just)" == 2 ]] || reject 'G0-F1 selector no longer occurs exactly twice'
  printf '%s\n' 'Task-4b Redis publish G0-F1 completion token is current'
}

verify_active_command_source_schema() {
  local source_file="$1"

  jq -e --arg generation_id "$active_generation" '
    .command_source_integrity.generation_id == $generation_id
    and (.command_source_integrity.canonical_order == [
      "scripts/tasks/task-13/r13-shape.just",
      "scripts/tasks/task-13/guarded-just-r13.sh",
      "tests/nix/flake-shape-aarch64-linux-r13.sh",
      "scripts/tasks/task-13/flake-source-snapshot-r13.sh"
    ])
    and ([.command_source_integrity.sources[].path] == .command_source_integrity.canonical_order)
    and (.command_source_integrity.sources | length == 4)
    and (all(.command_source_integrity.sources[];
      (.path | test("^(scripts/tasks/task-13/(r13-shape[.]just|guarded-just-r13[.]sh|flake-source-snapshot-r13[.]sh)|tests/nix/flake-shape-aarch64-linux-r13[.]sh)$"))
      and (.sha256 | test("^[0-9a-f]{64}$"))
    ))
    and (.command_source_integrity.post_validation_mutation_fixture.id == "active-r13-post-validation-live-justfile-mutation-zero-nested-dispatch")
    and (.command_source_integrity.post_validation_mutation_fixture.target == "scripts/tasks/task-13/r13-shape.just")
    and (.command_source_integrity.post_validation_mutation_fixture.mutations == ["content-mutation", "path-replacement"])
    and (.command_source_integrity.post_validation_mutation_fixture.expected_nested_dispatch_count == 0)
    and (.command_source_integrity.post_validation_mutation_fixture.expected_exit == 2)
    and (.command_source_integrity.reused_target_sources | length == 1)
    and (.command_source_integrity.reused_target_sources[0].path == "scripts/tasks/task-1/mod.just")
    and (.command_source_integrity.reused_target_sources[0].applies_to_recipes == ["flake-local", "flake-linux"])
    and (.command_source_integrity.reused_target_sources[0].sha256 | test("^[0-9a-f]{64}$"))
    and (.guarded_dispatch.execution_classes.immutable_snapshot == [
      {"justfile":"scripts/tasks/task-13/r13-shape.just","recipe":"aarch64-linux-shape-r13-immutable"},
      {"justfile":"scripts/tasks/task-1/mod.just","recipe":"flake-linux"},
      {"justfile":"scripts/tasks/task-1/mod.just","recipe":"flake-local"}
    ])
    and (.guarded_dispatch.execution_classes.live_worktree | length == 15)
    and (.guarded_dispatch.allowed_pairs | length == 18)
    and ((.guarded_dispatch.execution_classes.immutable_snapshot + .guarded_dispatch.execution_classes.live_worktree) as $classified
      | ($classified | length) == (.guarded_dispatch.allowed_pairs | length)
      and ([$classified[] | "\(.justfile)\u0000\(.recipe)"] | sort) == ([.guarded_dispatch.allowed_pairs[] | "\(.justfile)\u0000\(.recipe)"] | sort)
      and ([$classified[] | "\(.justfile)\u0000\(.recipe)"] | unique | length) == ($classified | length)
    )
  ' "$source_file" >/dev/null || reject 'active R13 command-source integrity schema is missing or incompatible'
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

  [[ "$source_root" == /* && -d "$source_root" ]] || reject "active R13 command-source root is invalid at ${phase}"
  [[ "$expected_sha" =~ ^[0-9a-f]{64}$ ]] || reject "active R13 command-source digest is malformed at ${phase}"
  case "$relative_path" in
    scripts/tasks/task-13/r13-shape.just | scripts/tasks/task-13/guarded-just-r13.sh | tests/nix/flake-shape-aarch64-linux-r13.sh | scripts/tasks/task-13/flake-source-snapshot-r13.sh | scripts/tasks/task-1/mod.just) ;;
    *) reject "active R13 command-source path is noncanonical at ${phase}" ;;
  esac
  source_directory="$source_root/$(dirname -- "$relative_path")"
  source_basename="$(basename -- "$relative_path")"
  [[ -d "$source_directory" ]] || reject "active R13 command-source directory is missing at ${phase}"
  physical_directory="$(cd -P -- "$source_directory" && pwd -P)" || reject "active R13 command-source directory cannot be resolved at ${phase}"
  physical_source="${physical_directory}/${source_basename}"
  [[ "$physical_source" == "$source_root/$relative_path" && -f "$physical_source" && ! -L "$physical_source" ]] || reject "active R13 command source is missing, linked, or escaped at ${phase}"
  actual_sha="$(sha256sum "$physical_source" | awk '{print $1}')"
  [[ "$actual_sha" == "$expected_sha" ]] || reject "active R13 command-source hash mismatch at ${phase}"
}

verify_command_sources_at_root() {
  local source_root="$1"
  local phase="$2"
  local source_file="${3:-$manifest_snapshot}"
  local expected_sha relative_path

  while IFS=$'\t' read -r expected_sha relative_path; do
    verify_recorded_source_at_root "$source_root" "$phase" "$expected_sha" "$relative_path"
  done < <(jq -r '.command_source_integrity.sources[] | "\(.sha256)\t\(.path)"' "$source_file")
}

verify_reused_task1_source_at_root() {
  local source_root="$1"
  local phase="$2"
  local source_file="${3:-$manifest_snapshot}"
  local expected_sha relative_path

  expected_sha="$(jq -er '.command_source_integrity.reused_target_sources[0].sha256' "$source_file")" || reject 'active R13 reused Task-1 source digest is missing'
  relative_path="$(jq -er '.command_source_integrity.reused_target_sources[0].path' "$source_file")" || reject 'active R13 reused Task-1 source path is missing'
  verify_recorded_source_at_root "$source_root" "$phase" "$expected_sha" "$relative_path"
}

allowed_pairs_from_snapshot() {
  jq -r '
    .guarded_dispatch.allowed_pairs[]
    | select(
        ((.justfile == "scripts/tasks/task-13/r13-shape.just") or (.justfile | test("^scripts/tasks/[A-Za-z0-9-]+/mod\\.just$")))
        and (.recipe | test("^[A-Za-z0-9][A-Za-z0-9-]*$"))
      )
    | "\(.justfile)\t\(.recipe)"
  ' "$manifest_snapshot" | LC_ALL=C sort -u
}

if [[ "$mode" == 'receipt' ]]; then
  verify_manifest_file "$manifest"
  verify_active_g0_prerequisite
  verify_active_command_source_schema "$manifest"
  verify_command_sources_at_root "$repo_root" 'receipt-live' "$manifest"
  printf 'task13_assertion_generation=%s\n' "$active_generation"
  printf 'task13_assertion_manifest_sha256=%s\n' "$snapshot_digest"
  printf 'coordinator record path=%s\n' "$fixed_record"
  printf '%s\n' 'DISPOSITION: ACTIVE_R13_MANIFEST_VERIFIED (zero mutation)'
  exit 0
fi

readonly recorded_digest target_justfile target_recipe
[[ "$recorded_digest" =~ ^[0-9a-f]{64}$ ]] || reject 'recorded digest must be lowercase 64-hex'
[[ "$target_justfile" == 'scripts/tasks/task-13/r13-shape.just' || "$target_justfile" =~ ^scripts/tasks/[A-Za-z0-9-]+/mod\.just$ ]] || reject 'justfile path is not canonical for active R13'
[[ "$target_recipe" =~ ^[A-Za-z0-9][A-Za-z0-9-]*$ ]] || reject 'recipe is not canonical'
verify_regular_repo_file "$target_justfile" 'R13 target Justfile'
manifest_snapshot="$(mktemp "${TMPDIR:-/tmp}/jamye-task13-r13-manifest.XXXXXX")" || reject 'cannot create R13 manifest snapshot'
cp -- "$manifest" "$manifest_snapshot" || reject 'cannot snapshot R13 manifest'
verify_manifest_file "$manifest_snapshot"

verify_inputs_against_snapshot
if ! allowed_pairs_from_snapshot | rg -Fqx -- "${target_justfile}"$'\t'"${target_recipe}"; then
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

verify_active_command_source_schema "$manifest_snapshot"
verify_active_g0_prerequisite
active_execution_class="$(active_execution_class_for_pair)" || reject 'active R13 dispatch pair classification failed'
verify_command_sources_at_root "$repo_root" 'live-before-dispatch'

if [[ "$active_execution_class" == 'immutable_snapshot' ]]; then
    if [[ "$target_justfile" == 'scripts/tasks/task-1/mod.just' ]]; then
      verify_reused_task1_source_at_root "$repo_root" 'live-before-snapshot'
    fi

    snapshot_helper_relative='scripts/tasks/task-13/flake-source-snapshot-r13.sh'
    snapshot_helper_expected_sha="$(jq -er --arg path "$snapshot_helper_relative" '.command_source_integrity.sources[] | select(.path == $path) | .sha256' "$manifest_snapshot")" || reject 'active R13 snapshot-helper hash is missing'
    snapshot_helper_copy="$(mktemp "${TMPDIR:-/tmp}/jamye-task13-r13-snapshot-helper.XXXXXX")" || reject 'cannot create verified snapshot-helper copy'
    cp -- "$repo_root/$snapshot_helper_relative" "$snapshot_helper_copy" || reject 'cannot copy the verified snapshot helper'
    [[ "$(sha256sum "$snapshot_helper_copy" | awk '{print $1}')" == "$snapshot_helper_expected_sha" ]] || reject 'verified snapshot-helper copy hash mismatch'

    if ! source_snapshot="$(TASK13_SOURCE_ROOT="$repo_root" bash "$snapshot_helper_copy")"; then
      reject 'filtered source snapshot creation failed'
    fi
    [[ "$source_snapshot" == /nix/store/* && "$source_snapshot" != *$'\n'* && -d "$source_snapshot" ]] || reject 'filtered source snapshot is not a Nix store directory'

    verify_inputs_against_snapshot
    verify_active_g0_prerequisite
    snapshot_manifest="$source_snapshot/$manifest"
    [[ -f "$snapshot_manifest" && ! -L "$snapshot_manifest" ]] || reject 'filtered source snapshot active manifest is missing or linked'
    cmp -s "$manifest_snapshot" "$snapshot_manifest" || reject 'filtered source snapshot active manifest differs from the verified manifest'
    [[ "$(sha256sum "$snapshot_manifest" | awk '{print $1}')" == "$snapshot_digest" ]] || reject 'filtered source snapshot active manifest digest mismatch'
    verify_active_command_source_schema "$snapshot_manifest"
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
    verify_active_g0_prerequisite

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

[[ "$active_execution_class" == 'live_worktree' ]] || reject 'active R13 dispatch pair has an invalid execution class'
verify_command_sources_at_root "$repo_root" 'live-before-worktree-dispatch'
verify_inputs_against_snapshot
verify_active_g0_prerequisite

printf 'task13_assertion_manifest_sha256=%s\n' "$snapshot_digest"
printf 'task13_assertion_generation=%s\n' "$generation_id"
# exec does not run Bash EXIT traps on success; remove the verified snapshot now.
rm -f -- "$manifest_snapshot"
manifest_snapshot=''
export TASK13_ASSERTION_GENERATION="$generation_id"
export TASK13_ASSERTION_DIGEST="$snapshot_digest"
exec just --justfile "$physical_justfile" "$target_recipe"
