#!/usr/bin/env bash
set -euo pipefail

readonly active_generation='task13-r21-g0-f9-frozen-coverage-entrypoint-20260907'
readonly manifest_relative='tests/nix/task-13-assertion-manifest-r21-g0-f9.json'
readonly lock_relative='tests/nix/task-13-assertion-manifest-r21-g0-f9.sha256'
readonly record_relative='.agents/results/task-13-s2-r21-g0-f9-manifest-digest-20260907.txt'
readonly authority_plan_relative='tests/nix/task-13-r21-authority-plan-20260907.json'
readonly authority_requirements_relative='tests/nix/task-13-r21-authority-requirements-20260907.md'
readonly token_relative='.agents/results/task-4b-redis-publish-completion-g0-f9-r21-20260907.json'
readonly token_review_relative='.agents/results/review-task13-g0-f9-r21-token-integrity-r1-20260907.md'
readonly entrypoint_relative='scripts/tasks/task-13/strict-entrypoint-r21.sh'
readonly guard_relative='scripts/tasks/task-13/guarded-just-r21.sh'
readonly shape_relative='scripts/tasks/task-13/r21-shape.just'
readonly snapshot_helper_relative='scripts/tasks/task-13/flake-source-snapshot-r21.sh'
readonly final_tree_helper_relative='scripts/tasks/task-13/final-tree-record-r21.sh'
readonly final_verify_helper_relative='scripts/tasks/task-13/final-verify-r21.sh'
readonly fixture_relative='tests/nix/coverage-executor-toctou-r21.sh'

readonly expected_authority_plan_sha='f04c4bb251a37bd05b827b39b7dc79eb15e283662ea7a1fabc131df866536d7f'
readonly expected_authority_requirements_sha='6c5f7b538986f5c47461eb552819e0320822d7444fbb90a7b2f5251edee7340f'
readonly expected_approval_sha='0fa3410b435521d3ad06fdfb44e3d08e3bd7030ee6f8d81af201969647ebf04d'
readonly expected_token_sha='30bc6ead1f1ae04d19fd38257dc46213c7f847d54b7fd2f9ecd0e60771d4422f'
readonly expected_token_review_sha='4af85af0e8f1774062e38a3fd1ff1604ec7b80a61c20f99c7405c5de2c84336d'
readonly expected_token_schema_sha='950f54d1e991760bb136fac605be224236b0ad32603ae594f836ca60167b6c3a'
readonly expected_token_template_sha='d36e798dc5b216f1e3d999ae399767a9c3db0bd7600c84c48ff32469adb3f81f'
readonly expected_coverage_stream_sha='bb77b0f773f339bc5f5767e57f96f6ad8352a299a53c1ff67f103595b4faa12f'
readonly completeness_review_path='.agents/results/review-task13-g0-f9-r21-plan-completeness-r5-20260907.md'
readonly completeness_review_sha='09bedd4f6b59b5aa4b5450613bff4d0e0e9ad0e270852e7b97bda58d6a3e4328'
readonly meta_review_path='.agents/results/review-task13-g0-f9-r21-plan-meta-r5-20260907.md'
readonly meta_review_sha='29d27d1a9ebf7a8bd367eebbc6c3f1be278d1fa8d7f1b352fc9f7ff68141c209'
readonly simplicity_review_path='.agents/results/review-task13-g0-f9-r21-plan-simplicity-r5-20260907.md'
readonly simplicity_review_sha='eda809ebc7032202420b8c5e677d11fe93e8e2af9e23345e032e4ab515b0876c'

manifest_snapshot=''
live_justfile_copy=''
snapshot_helper_copy=''
final_tree_helper_copy=''
final_verify_helper_copy=''

cleanup() {
  [[ -z "$manifest_snapshot" ]] || rm -f -- "$manifest_snapshot"
  [[ -z "$live_justfile_copy" ]] || rm -f -- "$live_justfile_copy"
  [[ -z "$snapshot_helper_copy" ]] || rm -f -- "$snapshot_helper_copy"
  [[ -z "$final_tree_helper_copy" ]] || rm -f -- "$final_tree_helper_copy"
  [[ -z "$final_verify_helper_copy" ]] || rm -f -- "$final_verify_helper_copy"
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
  printf '%s\n' 'usage: copied guarded-just-r21.sh task13-r21-g0-f9-frozen-coverage-entrypoint-20260907 (--receipt | <r21-recorded-digest> pair <justfile-relative-path> <private-recipe> [-- <recipe-args>])' >&2
  exit 2
}

sha256_file() {
  sha256sum "$1" | awk '{print $1}'
}

[[ -n "${IN_NIX_SHELL:-}" ]] || reject 'R21 guard requires the already-active repository Nix devShell'
[[ "${TASK13_R21_REPO_ROOT:-}" == /* && -d "${TASK13_R21_REPO_ROOT:-}" ]] || reject 'verified strict-entrypoint repository-root receipt is missing'
readonly repo_root="$(cd -P -- "$TASK13_R21_REPO_ROOT" && pwd -P)"
[[ "$repo_root" == "$TASK13_R21_REPO_ROOT" && "$(pwd -P)" == "$repo_root" ]] || reject 'R21 guard repository-root receipt is not physical or current'
[[ "${TASK13_R21_OPERATOR_MANIFEST_SHA256:-}" =~ ^[0-9a-f]{64}$ ]] || reject 'verified operator manifest receipt is missing'
[[ "${TASK13_R21_STRICT_ENTRYPOINT_SHA256:-}" =~ ^[0-9a-f]{64}$ ]] || reject 'verified strict-entrypoint digest receipt is missing'
[[ "${TASK13_R21_GUARD_SHA256:-}" =~ ^[0-9a-f]{64}$ ]] || reject 'verified guard digest receipt is missing'

readonly guard_invocation="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/$(basename -- "${BASH_SOURCE[0]}")"
readonly manifest_copy="$TASK13_R21_MANIFEST_COPY"
readonly strict_entrypoint_copy="$TASK13_R21_STRICT_ENTRYPOINT_COPY"
readonly operator_manifest_sha="$TASK13_R21_OPERATOR_MANIFEST_SHA256"
readonly strict_entrypoint_sha="$TASK13_R21_STRICT_ENTRYPOINT_SHA256"
readonly guard_sha="$TASK13_R21_GUARD_SHA256"

verify_external_copy() {
  local copy_path="$1"
  local expected_sha="$2"
  local phase="$3"
  local directory physical_directory physical_path
  [[ "$copy_path" == /* && -f "$copy_path" && ! -L "$copy_path" ]] || reject "strict bootstrap copy missing or linked at ${phase}"
  directory="$(dirname -- "$copy_path")"
  physical_directory="$(cd -P -- "$directory" && pwd -P)" || reject "strict bootstrap copy directory cannot be resolved at ${phase}"
  physical_path="${physical_directory}/$(basename -- "$copy_path")"
  [[ "$physical_path" == "$copy_path" ]] || reject "strict bootstrap copy path changed at ${phase}"
  case "$physical_path" in
    "$repo_root" | "$repo_root"/*) reject "strict bootstrap copy is inside the repository at ${phase}" ;;
  esac
  [[ "$(stat -c '%a' -- "$physical_path")" == 400 ]] || reject "strict bootstrap copy mode is not 0400 at ${phase}"
  [[ "$(sha256_file "$physical_path")" == "$expected_sha" ]] || reject "strict bootstrap copy digest mismatch at ${phase}"
}

verify_external_copy "$strict_entrypoint_copy" "$strict_entrypoint_sha" 'guard-entry'
verify_external_copy "$guard_invocation" "$guard_sha" 'guard-self'
[[ "$guard_invocation" == "$TASK13_R21_GUARD_COPY" ]] || reject 'guard is not the copied path selected by the strict entrypoint'
verify_external_copy "$manifest_copy" "$operator_manifest_sha" 'manifest-receipt'
cmp -s <(jq -cS '.' "$manifest_copy") "$manifest_copy" || reject 'verified R21 manifest copy is not canonical compact sorted JSON plus one LF'
readonly manifest_copy_identity="$(stat -c '%d:%i' -- "$manifest_copy")"
readonly guard_copy_identity="$(stat -c '%d:%i' -- "$guard_invocation")"

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

for required_path in "$manifest_relative" "$lock_relative" "$record_relative" "$authority_plan_relative" "$authority_requirements_relative" "$token_relative" "$token_review_relative"; do
  verify_repo_file "$required_path" 'R21 authority material'
done

readonly live_manifest="$repo_root/$manifest_relative"
readonly live_lock="$repo_root/$lock_relative"
readonly live_record="$repo_root/$record_relative"
readonly authority_plan="$repo_root/$authority_plan_relative"
readonly authority_requirements="$repo_root/$authority_requirements_relative"
readonly token="$repo_root/$token_relative"
readonly token_review="$repo_root/$token_review_relative"

verify_manifest_receipts() {
  [[ "$(stat -c '%d:%i' -- "$manifest_copy")" == "$manifest_copy_identity" ]] || reject 'verified manifest-copy identity changed'
  verify_external_copy "$manifest_copy" "$operator_manifest_sha" 'manifest-revalidation'
  [[ "$(sha256_file "$live_manifest")" == "$operator_manifest_sha" ]] || reject 'live R21 manifest differs from the operator receipt'
  cmp -s "$manifest_copy" "$live_manifest" || reject 'live R21 manifest differs from the verified copy'
  cmp -s <(printf '%s\n' "$operator_manifest_sha") "$live_lock" || reject 'R21 manifest lock differs from the operator receipt'
  cmp -s <(printf '%s\n' "$operator_manifest_sha") "$live_record" || reject 'R21 coordinator record differs from the operator receipt'
  [[ "$(stat -c '%d:%i' -- "$guard_invocation")" == "$guard_copy_identity" ]] || reject 'verified guard-copy identity changed'
  verify_external_copy "$guard_invocation" "$guard_sha" 'guard-revalidation'
  verify_external_copy "$strict_entrypoint_copy" "$strict_entrypoint_sha" 'entrypoint-revalidation'
}

verify_authority_prerequisite() {
  local completed_at_utc schema_sha template_sha coverage_stream_sha
  [[ "$(sha256_file "$authority_plan")" == "$expected_authority_plan_sha" ]] || reject 'stale R21 authority plan snapshot'
  [[ "$(sha256_file "$authority_requirements")" == "$expected_authority_requirements_sha" ]] || reject 'stale R21 authority requirements snapshot'
  [[ "$(tail -c 1 "$authority_plan" | od -An -tuC | tr -d '[:space:]')" == 10 ]] || reject 'R21 authority plan lacks final LF'
  [[ "$(tail -c 1 "$authority_requirements" | od -An -tuC | tr -d '[:space:]')" == 10 ]] || reject 'R21 authority requirements lack final LF'
  [[ "$(sha256_file "$token")" == "$expected_token_sha" ]] || reject 'stale R21 completion token'
  [[ "$(sha256_file "$token_review")" == "$expected_token_review_sha" ]] || reject 'stale R21 token-integrity review'
  [[ "$(rg -Fxc -- '## Review Result: PASS' "$token_review")" == 1 ]] || reject 'R21 token-integrity review is not the sole planned PASS review'
  cmp -s <(jq -cS '.' "$token") "$token" || reject 'R21 completion token is not canonical compact sorted JSON plus one LF'

  schema_sha="$(jq -cS '.assertion_manifests.task13_r21.token_schema' "$authority_plan" | sha256sum | awk '{print $1}')"
  template_sha="$(jq -cS '.assertion_manifests.task13_r21.token_schema.token_template' "$authority_plan" | sha256sum | awk '{print $1}')"
  [[ "$schema_sha" == "$expected_token_schema_sha" ]] || reject 'R21 materialized token schema hash mismatch'
  [[ "$template_sha" == "$expected_token_template_sha" ]] || reject 'R21 token template hash mismatch'

  jq -e \
    --arg generation "$active_generation" \
    --arg completeness "$completeness_review_path" \
    --arg meta "$meta_review_path" \
    --arg simplicity "$simplicity_review_path" \
    --arg post_token "$token_review_relative" '
      .assertion_manifests.task13_r21.token_schema == .assertion_manifests.task13_r21.manifest_projection.token_schema
      and .assertion_manifests.task13_r21.generation == $generation
      and .assertion_manifests.task13_r21.generation_argument_invariant.assertion_generation == $generation
      and (.assertion_manifests.task13_r21.guarded_dispatch.arguments | split(" ")[0]) == $generation
      and [.assertion_manifests.task13_r21.reviews.plan[].path] == [$completeness,$meta,$simplicity]
      and .assertion_manifests.task13_r21.reviews.sole_post_token.path == $post_token
    ' "$authority_plan" >/dev/null || reject 'R21 plan schema, reviews, or generation binding differs'

  completed_at_utc="$(jq -er '.completed_at_utc' "$token")" || reject 'R21 token issuance time is missing'
  [[ "$completed_at_utc" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$ ]] || reject 'R21 token issuance time is malformed'
  jq -e \
    --slurpfile plan "$authority_plan" \
    --arg completed_at_utc "$completed_at_utc" \
    --arg plan_sha "$expected_authority_plan_sha" \
    --arg requirements_sha "$expected_authority_requirements_sha" \
    --arg approval_sha "$expected_approval_sha" \
    --arg completeness_path "$completeness_review_path" \
    --arg completeness_sha "$completeness_review_sha" \
    --arg meta_path "$meta_review_path" \
    --arg meta_sha "$meta_review_sha" \
    --arg simplicity_path "$simplicity_review_path" \
    --arg simplicity_sha "$simplicity_review_sha" '
      def instantiate:
        walk(
          if type != "string" then .
          elif . == "<completed-at-utc>" then $completed_at_utc
          elif . == "<plan-sha256>" then $plan_sha
          elif . == "<requirements-sha256>" then $requirements_sha
          elif . == "<approval-sha256>" then $approval_sha
          elif . == "<completeness-review-path>" then $completeness_path
          elif . == "<completeness-review-sha256>" then $completeness_sha
          elif . == "<meta-review-path>" then $meta_path
          elif . == "<meta-review-sha256>" then $meta_sha
          elif . == "<simplicity-review-path>" then $simplicity_path
          elif . == "<simplicity-review-sha256>" then $simplicity_sha
          else . end
        );
      def normalized_object_key_sets:
        reduce (paths(objects) as $path
          | {key:("/" + ($path | map(if type == "number" then "*" else tostring end) | join("/"))), value:(getpath($path) | keys)})
          as $entry ({};
            if has($entry.key) and .[$entry.key] != $entry.value
            then error("nonuniform normalized object key set")
            else .[$entry.key] = $entry.value end);
      ($plan[0].assertion_manifests.task13_r21.token_schema) as $schema
      | ($schema.token_template | instantiate) as $expected
      | (keys == $schema.top_level_keys)
        and (normalized_object_key_sets == $schema.nested_object_keys)
        and (. == $expected)
        and ([.. | select(. == null)] | length == 0)
        and ([.. | strings | select(. as $value | any($schema.placeholder_rules | keys[]; . == $value))] | length == 0)
    ' "$token" >/dev/null || reject 'R21 completion token fails the complete materialized schema'

  coverage_stream_sha="$(jq -r '.assertion_manifests.task13_r21.coverage.green.ordered_commands[]' "$authority_plan" | sha256sum | awk '{print $1}')"
  [[ "$coverage_stream_sha" == "$expected_coverage_stream_sha" ]] || reject 'R21 coverage command stream hash mismatch'
}

verify_manifest_protocol() {
  jq -e --slurpfile plan "$authority_plan" --arg generation "$active_generation" --arg manifest "$manifest_relative" --arg lock "$lock_relative" --arg record "$record_relative" --arg plan_path "$authority_plan_relative" --arg plan_sha "$expected_authority_plan_sha" --arg requirements_path "$authority_requirements_relative" --arg requirements_sha "$expected_authority_requirements_sha" --arg token_path "$token_relative" --arg token_sha "$expected_token_sha" --arg review_path "$token_review_relative" --arg review_sha "$expected_token_review_sha" '
    def pair_object: split("::") | {justfile:.[0], recipe:.[1]};
    . as $manifest_object
    | $plan[0].assertion_manifests.task13_r21 as $assertion
    | $assertion.manifest_projection as $projection
    | ($assertion.guarded_dispatch.allowed_pairs | map(pair_object)) as $allowed
    | ($assertion.guarded_dispatch.execution_classes.immutable_snapshot | map(pair_object)) as $immutable
    | ($assertion.guarded_dispatch.execution_classes.live_worktree | map(pair_object)) as $live
    | .generation_id == $generation
      and .owned_path == $manifest
      and .guarded_dispatch.generation_id == $generation
      and .guarded_dispatch.adapter == "scripts/tasks/task-13/guarded-just-r21.sh"
      and .guarded_dispatch.allowed_pairs == $allowed
      and .guarded_dispatch.execution_classes.immutable_snapshot == $immutable
      and .guarded_dispatch.execution_classes.live_worktree == $live
      and .guarded_dispatch.pair_argument_schemas == $assertion.guarded_dispatch.pair_argument_schemas
      and ($allowed | length == 24 and unique | length == 24)
      and ($immutable | length == 3 and unique | length == 3)
      and ($live | length == 21 and unique | length == 21)
      and (($immutable + $live) | unique) == ($allowed | unique)
      and ([ $immutable[] as $pair | select(($live | index($pair)) != null) ] | length == 0)
      and .authority_binding.generation_id == $generation
      and .authority_binding.runtime_resolution == "immutable_snapshot_only"
      and .authority_binding.runtime_mutable_canonical_access == "forbidden"
      and .authority_binding.snapshots.approved_plan == {path:$plan_path,sha256:$plan_sha}
      and .authority_binding.snapshots.approved_requirements == {path:$requirements_path,sha256:$requirements_sha}
      and .authority_binding.token.path == $token_path
      and .authority_binding.token.sha256 == $token_sha
      and .authority_binding.token_integrity_review == {path:$review_path,sha256:$review_sha,status:"PASS"}
      and .authority_binding.active_assertion_contract == $assertion
      and .authority_binding.active_protocol_authority == $plan[0].active_r21_final_protocol_authority
      and all(($projection | keys[]); . as $key | $manifest_object[$key] == $projection[$key])
  ' "$manifest_copy" >/dev/null || reject 'active R21 manifest projection, pair partition, or authority binding is invalid'

  jq -e '
    .operator_surfaces.final_verify as $fv
    | .final_tree_snapshot.catalog_dispatch_entries as $catalog
    | ["fv-01-pf1-fingerprint","fv-02-operation-event-matrix","fv-03-consolidated-whole-tree","fv-04-cross-feature-recovery-privacy-lease-voice","fv-05-migration-adr-chain","fv-06-c2-provenance","fv-07-supported-system-builder-matrix","fv-08-dependency-license-log-gitleaks"] as $ids
    | ([ $catalog[].dispatch_id ] == $ids)
      and ([ $fv.dispatch_by_id | keys[] | select(. != "fv-09-redacted-no-mutation-closeout") ] == $ids)
      and ([ $fv.inventory[].id | select(. != "fv-09-redacted-no-mutation-closeout") ] == $ids)
      and all($ids[]; . as $id
        | ($catalog[] | select(.dispatch_id == $id)) as $catalog_entry
        | ($fv.inventory[] | select(.id == $id)) as $inventory_entry
        | ($catalog_entry.exact_ordered_command_strings | type) == "array"
          and ($catalog_entry.internal_ordered_command_strings | type) == "array"
          and $catalog_entry.exact_ordered_command_strings == $fv.dispatch_by_id[$id].dispatch
          and $catalog_entry.exact_ordered_command_strings == $inventory_entry.dispatch
          and $catalog_entry.internal_ordered_command_strings == $fv.dispatch_by_id[$id].internal_dispatch
          and $catalog_entry.internal_ordered_command_strings == $inventory_entry.internal_dispatch)
  ' "$manifest_copy" >/dev/null || reject 'R21 FV catalog, dispatch_by_id, and inventory arrays are not exactly equal'

  [[ "$(jq -cS '.token_schema' "$manifest_copy" | sha256sum | awk '{print $1}')" == "$expected_token_schema_sha" ]] || reject 'active R21 manifest token schema hash mismatch'
  [[ "$(jq -cS '.token_schema.token_template' "$manifest_copy" | sha256sum | awk '{print $1}')" == "$expected_token_template_sha" ]] || reject 'active R21 manifest token template hash mismatch'
  [[ "$(jq -r '.coverage.green.ordered_commands[]' "$manifest_copy" | sha256sum | awk '{print $1}')" == "$expected_coverage_stream_sha" ]] || reject 'active R21 manifest coverage stream hash mismatch'
}

verify_command_source_schema() {
  jq -e --arg generation "$active_generation" --arg fixture "$fixture_relative" '
    .command_source_integrity.generation_id == $generation
    and .command_source_integrity.canonical_order == [
      "scripts/tasks/task-13/r21-shape.just",
      "scripts/tasks/task-13/guarded-just-r21.sh",
      "scripts/tasks/task-13/strict-entrypoint-r21.sh",
      "tests/nix/flake-shape-aarch64-linux-r21.sh",
      "scripts/tasks/task-13/flake-source-snapshot-r21.sh",
      "scripts/tasks/task-13/final-tree-record-r21.sh",
      "scripts/tasks/task-13/final-verify-r21.sh",
      "tests/nix/coverage-executor-toctou-r21.sh"
    ]
    and [.command_source_integrity.sources[].path] == .command_source_integrity.canonical_order
    and (.command_source_integrity.sources | length == 8)
    and (all(.command_source_integrity.sources[]; (.sha256 | test("^[0-9a-f]{64}$"))))
    and .command_source_integrity.live_auxiliary_dependencies == []
    and .command_source_integrity.post_validation_mutation_fixture.executable_path == $fixture
    and (.command_source_integrity.post_validation_mutation_fixture.executable_sha256 | test("^[0-9a-f]{64}$"))
    and .command_source_integrity.post_validation_mutation_fixture.executable_sha256 == ([.command_source_integrity.sources[] | select(.path == $fixture)][0].sha256)
    and .command_source_integrity.post_validation_mutation_fixture.expected_exit == 2
    and .command_source_integrity.post_validation_mutation_fixture.expected_nested_dispatch_count == 0
    and .command_source_integrity.post_validation_mutation_fixture.expected_repository_persistent_write_count == 0
  ' "$manifest_copy" >/dev/null || reject 'active R21 command-source or fixture schema is invalid'
}

verify_source_at_root() {
  local source_root="$1"
  local relative_path="$2"
  local expected_sha="$3"
  local phase="$4"
  local directory physical_directory physical_path
  [[ "$source_root" == /* && -d "$source_root" ]] || reject "source root is invalid at ${phase}"
  [[ "$expected_sha" =~ ^[0-9a-f]{64}$ ]] || reject "source digest is malformed at ${phase}"
  case "$relative_path" in
    "$shape_relative"|"$guard_relative"|"$entrypoint_relative"|'tests/nix/flake-shape-aarch64-linux-r21.sh'|"$snapshot_helper_relative"|"$final_tree_helper_relative"|"$final_verify_helper_relative"|"$fixture_relative"|scripts/tasks/task-1/mod.just|"$authority_plan_relative"|"$authority_requirements_relative"|"$token_relative"|"$token_review_relative") ;;
    *) reject "source path is not an active R21 runtime input at ${phase}" ;;
  esac
  directory="$source_root/$(dirname -- "$relative_path")"
  [[ -d "$directory" ]] || reject "source directory is missing at ${phase}"
  physical_directory="$(cd -P -- "$directory" && pwd -P)" || reject "source directory cannot be resolved at ${phase}"
  physical_path="${physical_directory}/$(basename -- "$relative_path")"
  [[ "$physical_path" == "$source_root/$relative_path" && -f "$physical_path" && ! -L "$physical_path" ]] || reject "source is missing, linked, or escaped at ${phase}"
  [[ "$(sha256_file "$physical_path")" == "$expected_sha" ]] || reject "manifest-bound source hash mismatch at ${phase}: ${relative_path}"
}

verify_frozen_sources_at_root() {
  local source_root="$1"
  local phase="$2"
  local source_sha source_path
  while IFS=$'\t' read -r source_sha source_path; do
    verify_source_at_root "$source_root" "$source_path" "$source_sha" "$phase"
  done < <(jq -r '.command_source_integrity.sources[] | "\(.sha256)\t\(.path)"' "$manifest_copy")
}

verify_reused_task1_at_root() {
  local source_root="$1"
  local phase="$2"
  local reused_path reused_sha
  reused_path="$(jq -er '.command_source_integrity.reused_target_sources | if length == 1 then .[0].path else error("expected one reused source") end' "$manifest_copy")" || reject 'R21 reused Task-1 source path is missing'
  reused_sha="$(jq -er '.command_source_integrity.reused_target_sources[0].sha256' "$manifest_copy")" || reject 'R21 reused Task-1 source digest is missing'
  [[ "$reused_path" == 'scripts/tasks/task-1/mod.just' ]] || reject 'R21 reused immutable source is not Task-1 mod.just'
  jq -e '.command_source_integrity.reused_target_sources[0].applies_to_recipes == ["flake-local","flake-linux"]' "$manifest_copy" >/dev/null || reject 'R21 reused Task-1 recipe binding is invalid'
  verify_source_at_root "$source_root" "$reused_path" "$reused_sha" "$phase"
}

verify_everything_live() {
  verify_manifest_receipts
  verify_authority_prerequisite
  verify_manifest_protocol
  verify_command_source_schema
  verify_frozen_sources_at_root "$repo_root" "$1"
}

active_execution_class() {
  jq -er --arg justfile "$target_justfile" --arg recipe "$target_recipe" '
    [.guarded_dispatch.execution_classes | to_entries[]
      | select(any(.value[]; .justfile == $justfile and .recipe == $recipe)) | .key]
    | if length == 1 then .[0] else error("pair classification must be exact") end
  ' "$manifest_copy"
}

validate_recipe_arguments() {
  local pair="${target_justfile}::${target_recipe}"
  case "$pair" in
    "$shape_relative::module-eval-r21-exec")
      ((${#recipe_arguments[@]} == 1)) || reject 'module evaluation requires exactly one phase argument'
      [[ "${recipe_arguments[0]}" == 's2-skeleton' || "${recipe_arguments[0]}" == 's4-green' || "${recipe_arguments[0]}" == 's5-final' ]] || reject 'module evaluation phase is not allowlisted'
      ;;
    "$shape_relative::linux-builder-r21-exec"|"$shape_relative::aarch64-linux-builder-r21-exec"|"$shape_relative::cross-system-verify-r21-exec")
      ((${#recipe_arguments[@]} == 1 || ${#recipe_arguments[@]} == 2)) || reject 'builder or cross-system dispatch requires a GPE-map digest and optional fv07-replay'
      [[ "${recipe_arguments[0]}" =~ ^[0-9a-f]{64}$ ]] || reject 'GPE-map digest must be lowercase 64-hex'
      ((${#recipe_arguments[@]} == 1)) || [[ "${recipe_arguments[1]}" == 'fv07-replay' ]] || reject 'optional replay argument must be literal fv07-replay'
      ;;
    "$shape_relative::final-verify-r21-exec")
      ((${#recipe_arguments[@]} == 2)) || reject 'final verify requires exactly one FV01-FV08 dispatch ID and one GPE-map digest'
      case "${recipe_arguments[0]}" in
        fv-01-pf1-fingerprint|fv-02-operation-event-matrix|fv-03-consolidated-whole-tree|fv-04-cross-feature-recovery-privacy-lease-voice|fv-05-migration-adr-chain|fv-06-c2-provenance|fv-07-supported-system-builder-matrix|fv-08-dependency-license-log-gitleaks) ;;
        *) reject 'final verify dispatch ID is not one of FV01-FV08' ;;
      esac
      [[ "${recipe_arguments[1]}" =~ ^[0-9a-f]{64}$ ]] || reject 'final verify GPE-map digest must be lowercase 64-hex'
      ;;
    *)
      ((${#recipe_arguments[@]} == 0)) || reject 'selected pair accepts zero recipe arguments'
      ;;
  esac
}

wait_at_post_copy_fixture_barrier() {
  local fixture_mode="${TASK13_R21_POST_COPY_FIXTURE_MODE:-}"
  local fixture_root="${TASK13_R21_FIXTURE_REPO_ROOT:-}"
  local ready_fd="${TASK13_R21_FIXTURE_READY_FD:-}"
  local release_fd="${TASK13_R21_FIXTURE_RELEASE_FD:-}"
  local temp_root release
  if [[ -z "$fixture_mode$fixture_root$ready_fd$release_fd" ]]; then
    return 0
  fi
  [[ "$target_justfile" == "$shape_relative" && "$target_recipe" == 'coverage-r21-exec' ]] || reject 'R21 post-copy fixture is restricted to the frozen coverage pair'
  [[ "$fixture_mode" == 'post-copy-content-mutation' || "$fixture_mode" == 'post-copy-path-replacement' ]] || reject 'R21 post-copy fixture mode is invalid'
  [[ "$fixture_root" == "$repo_root" ]] || reject 'R21 post-copy fixture root does not match the guarded repository'
  temp_root="$(cd -P -- "${TMPDIR:-/tmp}" && pwd -P)" || reject 'R21 fixture temp root cannot be resolved'
  case "$repo_root" in
    "$temp_root"/jamye-task13-r21-toctou.*/repo) ;;
    *) reject 'R21 post-copy fixture is forbidden outside its disposable repository' ;;
  esac
  [[ "$ready_fd" =~ ^[0-9]+$ && "$release_fd" =~ ^[0-9]+$ && "$ready_fd" != "$release_fd" ]] || reject 'R21 fixture descriptors are invalid'
  printf 'ready:%s\n' "$fixture_mode" >&"$ready_fd" || reject 'R21 fixture ready signal failed'
  IFS= read -r release <&"$release_fd" || reject 'R21 fixture release signal failed'
  [[ "$release" == "release:${fixture_mode}" ]] || reject 'R21 fixture release token is invalid'
}

copy_manifest_bound_helper() {
  local relative_path="$1"
  local name="$2"
  local expected_sha destination
  expected_sha="$(jq -er --arg path "$relative_path" '.command_source_integrity.sources[] | select(.path == $path) | .sha256' "$manifest_copy")" || reject "manifest digest missing for ${relative_path}"
  destination="$(mktemp "${TMPDIR:-/tmp}/jamye-task13-r21-${name}.XXXXXX")" || reject "cannot create ${name} copy"
  cp -- "$repo_root/$relative_path" "$destination" || reject "cannot copy ${relative_path}"
  chmod 0400 -- "$destination" || reject "cannot make ${name} copy read-only"
  destination="$(cd -P -- "$(dirname -- "$destination")" && pwd -P)/$(basename -- "$destination")"
  [[ "$(stat -c '%a' -- "$destination")" == 400 && ! -L "$destination" && "$(sha256_file "$destination")" == "$expected_sha" ]] || reject "verified ${name} copy differs from the manifest-bound source"
  COPIED_HELPER_PATH="$destination"
  COPIED_HELPER_SHA="$expected_sha"
}

(( $# == 2 || $# >= 5 )) || usage
generation_id="$1"
[[ "$generation_id" == "$active_generation" ]] || reject 'unknown Task-13 R21 assertion generation'
if (($# == 2)); then
  [[ "$2" == '--receipt' ]] || usage
  mode='receipt'
else
  recorded_digest="$2"
  [[ "$3" == 'pair' ]] || usage
  target_justfile="$4"
  target_recipe="$5"
  shift 5
  recipe_arguments=()
  if (($# > 0)); then
    [[ "$1" == '--' ]] || reject 'recipe arguments require the literal -- separator'
    shift
    (($# > 0)) || reject 'the recipe argument separator cannot be empty'
    recipe_arguments=("$@")
  fi
  mode='dispatch'
  readonly recorded_digest target_justfile target_recipe
  readonly -a recipe_arguments
fi
readonly generation_id mode

verify_everything_live 'guard-entry'

if [[ "$mode" == 'receipt' ]]; then
  printf 'task13_assertion_generation=%s\n' "$active_generation"
  printf 'task13_assertion_manifest_sha256=%s\n' "$operator_manifest_sha"
  printf 'task13_strict_entrypoint_sha256=%s\n' "$strict_entrypoint_sha"
  printf 'coordinator record path=%s\n' "$record_relative"
  printf '%s\n' 'DISPOSITION: ACTIVE_R21_MANIFEST_VERIFIED (zero mutation)'
  exit 0
fi

[[ "$recorded_digest" =~ ^[0-9a-f]{64}$ && "$recorded_digest" == "$operator_manifest_sha" ]] || reject 'caller manifest digest differs from the strict operator receipt'
[[ "$target_justfile" == "$shape_relative" || "$target_justfile" =~ ^scripts/tasks/[A-Za-z0-9-]+/mod\.just$ ]] || reject 'Justfile path is not canonical for active R21'
[[ "$target_recipe" =~ ^[A-Za-z0-9][A-Za-z0-9-]*$ ]] || reject 'recipe is not canonical for active R21'
validate_recipe_arguments
if ! jq -er --arg justfile "$target_justfile" --arg recipe "$target_recipe" 'any(.guarded_dispatch.allowed_pairs[]; .justfile == $justfile and .recipe == $recipe)' "$manifest_copy" >/dev/null; then
  reject 'Justfile and recipe pair is not allowlisted by the verified R21 manifest'
fi

verify_repo_file "$target_justfile" 'selected R21 Justfile'
execution_class="$(active_execution_class)" || reject 'R21 pair execution classification failed'
readonly execution_class
verify_everything_live 'before-pair-dispatch'

target_directory="$repo_root/$(dirname -- "$target_justfile")"
physical_directory="$(cd -P -- "$target_directory" && pwd -P)" || reject 'selected Justfile directory cannot be resolved'
physical_justfile="${physical_directory}/$(basename -- "$target_justfile")"
[[ "$physical_justfile" == "$repo_root/$target_justfile" && -f "$physical_justfile" && ! -L "$physical_justfile" ]] || reject 'selected Justfile is missing, linked, or escaped'

if [[ "$execution_class" == 'immutable_snapshot' ]]; then
  if [[ "$target_justfile" == 'scripts/tasks/task-1/mod.just' ]]; then
    verify_reused_task1_at_root "$repo_root" 'live-before-snapshot'
  fi
  copy_manifest_bound_helper "$snapshot_helper_relative" 'snapshot-helper'
  snapshot_helper_copy="$COPIED_HELPER_PATH"
  snapshot_helper_sha="$COPIED_HELPER_SHA"
  if ! source_snapshot="$(TASK13_SOURCE_ROOT="$repo_root" TASK13_R21_REPO_ROOT="$repo_root" TASK13_R21_MANIFEST_COPY="$manifest_copy" TASK13_R21_SNAPSHOT_HELPER_SHA256="$snapshot_helper_sha" bash "$snapshot_helper_copy")"; then
    reject 'R21 filtered source snapshot creation failed'
  fi
  [[ "$source_snapshot" == /nix/store/* && "$source_snapshot" != *$'\n'* && -d "$source_snapshot" ]] || reject 'R21 filtered source snapshot is not a Nix store directory'
  verify_everything_live 'live-after-snapshot'
  snapshot_manifest="$source_snapshot/$manifest_relative"
  [[ -f "$snapshot_manifest" && ! -L "$snapshot_manifest" ]] || reject 'R21 snapshot manifest is missing or linked'
  cmp -s "$manifest_copy" "$snapshot_manifest" || reject 'R21 snapshot manifest differs from the verified manifest copy'
  verify_frozen_sources_at_root "$source_snapshot" 'immutable-snapshot'
  if [[ "$target_justfile" == 'scripts/tasks/task-1/mod.just' ]]; then
    verify_reused_task1_at_root "$source_snapshot" 'immutable-snapshot'
  fi
  snapshot_target="$source_snapshot/$target_justfile"
  [[ -f "$snapshot_target" && ! -L "$snapshot_target" ]] || reject 'R21 snapshot target Justfile is missing or linked'
  cmp -s "$physical_justfile" "$snapshot_target" || reject 'R21 snapshot target differs from the validated live target'
  verify_everything_live 'immutable-final-pre-dispatch'
  printf 'task13_assertion_generation=%s\n' "$active_generation"
  printf 'task13_assertion_manifest_sha256=%s\n' "$operator_manifest_sha"
  printf 'task13_flake_source_snapshot=%s\n' "$source_snapshot"
  export TASK13_ASSERTION_GENERATION="$active_generation"
  export TASK13_ASSERTION_DIGEST="$operator_manifest_sha"
  export TASK13_FLAKE_SOURCE_SNAPSHOT="$source_snapshot"
  set +e
  (cd "$source_snapshot" && just --justfile "$snapshot_target" "$target_recipe" "${recipe_arguments[@]}")
  nested_status=$?
  set -e
  exit "$nested_status"
fi

[[ "$execution_class" == 'live_worktree' ]] || reject 'R21 pair has an invalid execution class'
source_identity="$(stat -c '%d:%i' -- "$physical_justfile")"
if [[ "$target_justfile" == "$shape_relative" ]]; then
  source_sha="$(jq -er --arg path "$shape_relative" '.command_source_integrity.sources[] | select(.path == $path) | .sha256' "$manifest_copy")" || reject 'manifest-bound r21-shape digest is missing'
  [[ "$(sha256_file "$physical_justfile")" == "$source_sha" ]] || reject 'selected r21-shape source differs from the manifest before copy'
else
  source_sha="$(sha256_file "$physical_justfile")"
fi

live_justfile_copy="$(mktemp "$physical_directory/.jamye-task13-r21-live-justfile.XXXXXX")" || reject 'cannot create adjacent selected Justfile copy'
cp -- "$physical_justfile" "$live_justfile_copy" || reject 'cannot copy selected Justfile'
chmod 0400 -- "$live_justfile_copy" || reject 'cannot make selected Justfile copy read-only'
live_justfile_copy="$(cd -P -- "$(dirname -- "$live_justfile_copy")" && pwd -P)/$(basename -- "$live_justfile_copy")"
copy_identity="$(stat -c '%d:%i' -- "$live_justfile_copy")"
[[ "$(sha256_file "$live_justfile_copy")" == "$source_sha" ]] || reject 'adjacent selected Justfile copy differs from approved bytes'

copy_manifest_bound_helper "$final_tree_helper_relative" 'final-tree-helper'
final_tree_helper_copy="$COPIED_HELPER_PATH"
final_tree_helper_sha="$COPIED_HELPER_SHA"
copy_manifest_bound_helper "$final_verify_helper_relative" 'final-verify-helper'
final_verify_helper_copy="$COPIED_HELPER_PATH"
final_verify_helper_sha="$COPIED_HELPER_SHA"

verify_everything_live 'live-before-fixture-barrier'
wait_at_post_copy_fixture_barrier
verify_everything_live 'live-final-pre-dispatch'
verify_repo_file "$target_justfile" 'selected Justfile final validation'
[[ "$(stat -c '%d:%i' -- "$physical_justfile")" == "$source_identity" ]] || reject 'bound source identity changed at live-target-final-pre-dispatch'
[[ "$(sha256_file "$physical_justfile")" == "$source_sha" ]] || reject 'bound source content changed at live-target-final-pre-dispatch'
[[ -f "$live_justfile_copy" && ! -L "$live_justfile_copy" && "$(stat -c '%a' -- "$live_justfile_copy")" == 400 ]] || reject 'bound read-only Justfile copy changed at live-target-final-pre-dispatch'
[[ "$(stat -c '%d:%i' -- "$live_justfile_copy")" == "$copy_identity" ]] || reject 'bound read-only Justfile copy identity changed at live-target-final-pre-dispatch'
[[ "$(sha256_file "$live_justfile_copy")" == "$source_sha" ]] || reject 'bound read-only Justfile copy content changed at live-target-final-pre-dispatch'
[[ "$(sha256_file "$final_tree_helper_copy")" == "$final_tree_helper_sha" ]] || reject 'final-tree helper copy changed before dispatch'
[[ "$(sha256_file "$final_verify_helper_copy")" == "$final_verify_helper_sha" ]] || reject 'final-verify helper copy changed before dispatch'

export TASK13_ASSERTION_GENERATION="$active_generation"
export TASK13_ASSERTION_DIGEST="$operator_manifest_sha"
export TASK13_R21_ASSERTION_GENERATION="$active_generation"
export TASK13_R21_ASSERTION_DIGEST="$operator_manifest_sha"
export TASK13_R21_BOUND_JUSTFILE="$live_justfile_copy"
export TASK13_R21_BOUND_JUSTFILE_IDENTITY="$copy_identity"
export TASK13_R21_BOUND_JUSTFILE_SHA256="$source_sha"
export TASK13_R21_FINAL_TREE_HELPER_COPY="$final_tree_helper_copy"
export TASK13_R21_FINAL_TREE_HELPER_SHA256="$final_tree_helper_sha"
export TASK13_R21_FINAL_VERIFY_HELPER_COPY="$final_verify_helper_copy"
export TASK13_R21_FINAL_VERIFY_HELPER_SHA256="$final_verify_helper_sha"
unset TASK13_R21_GPE_MAP_SHA256 TASK13_R21_FV_DISPATCH_ID TASK13_R21_REPLAY_MODE
case "$target_recipe" in
  linux-builder-r21-exec|aarch64-linux-builder-r21-exec|cross-system-verify-r21-exec)
    export TASK13_R21_GPE_MAP_SHA256="${recipe_arguments[0]}"
    ((${#recipe_arguments[@]} == 1)) || export TASK13_R21_REPLAY_MODE='fv07-replay'
    ;;
  final-verify-r21-exec)
    export TASK13_R21_FV_DISPATCH_ID="${recipe_arguments[0]}"
    export TASK13_R21_GPE_MAP_SHA256="${recipe_arguments[1]}"
    ;;
esac

printf 'task13_assertion_manifest_sha256=%s\n' "$operator_manifest_sha"
printf 'task13_assertion_generation=%s\n' "$active_generation"
set +e
just --justfile "$live_justfile_copy" "$target_recipe" "${recipe_arguments[@]}"
nested_status=$?
set -e
exit "$nested_status"
