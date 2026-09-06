#!/usr/bin/env bash
set -euo pipefail
readonly active_generation='task13-r14-g0-f2-authority-snapshots-20260905'
readonly manifest='tests/nix/task-13-assertion-manifest-r14-g0-f2.json'
readonly lock='tests/nix/task-13-assertion-manifest-r14-g0-f2.sha256'
readonly fixed_record='.agents/results/task-13-s2-r14-g0-f2-manifest-digest-20260905.txt'
readonly g0_token='.agents/results/task-4b-redis-publish-completion-g0-f2-r14-20260905.json'
readonly expected_g0_token_sha='5500b6a5fd73d376d19d7bd0fa95ecaeecf9e83e6b9002186603623e15656811'
readonly g0_token_review='.agents/results/review-task13-g0-f2-r14-token-integrity-r1-20260905.md'
readonly expected_g0_token_review_sha='8ff005d0bddff58a95efe77a6b21986cd1cff1d7eb8c065e27f41670af72f1b3'
readonly authority_plan='tests/nix/task-13-r14-authority-plan-20260905.json'
readonly expected_authority_plan_sha='3ce944473c35b916e6e981a0d1d8ac981d19a7aafdbf39ec75e2ada2c98e25f2'
readonly authority_requirements='tests/nix/task-13-r14-authority-requirements-20260905.md'
readonly expected_authority_requirements_sha='bdcf0ac3e5ebf760c4ba055b986a2b2ead2a5942141aefd4ed1410fd91a6bc0f'
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
  printf '%s\n' 'usage: guarded-just-r14.sh task13-r14-g0-f2-authority-snapshots-20260905 (--receipt | <recorded-digest> <justfile-relative-path> <recipe>)' >&2
  exit 2
fi
readonly mode generation_id
manifest_snapshot=''
snapshot_helper_copy=''
live_justfile_copy=''

cleanup() {
  [[ -z "$manifest_snapshot" ]] || rm -f -- "$manifest_snapshot"
  [[ -z "$snapshot_helper_copy" ]] || rm -f -- "$snapshot_helper_copy"
  [[ -z "$live_justfile_copy" ]] || rm -f -- "$live_justfile_copy"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
reject() {
  printf 'error: %s\n' "$1" >&2
  exit 2
}
[[ "$(pwd -P)" == "$repo_root" ]] || reject 'R14 guard must start at the repository root'
[[ "$generation_id" == "$active_generation" ]] || reject 'unknown Task-13 R14 assertion generation'
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
  verify_regular_repo_file "$required_path" 'R14 manifest material'
done
verify_manifest_file() {
  local source_file="$1"
  local expected
  cmp -s <(jq -cS '.' "$source_file") "$source_file" || reject 'R14 manifest bytes are not canonical JSON plus LF'
  snapshot_digest="$(sha256sum "$source_file" | awk '{print $1}')"
  [[ "$snapshot_digest" =~ ^[0-9a-f]{64}$ ]] || reject 'R14 manifest digest is malformed'
  expected="${snapshot_digest}"$'\n'
  cmp -s <(printf '%s' "$expected") "$lock" || reject 'R14 manifest lock is malformed or mismatched'
  cmp -s <(printf '%s' "$expected") "$fixed_record" || reject 'R14 coordinator record is malformed or mismatched'
  jq -e --arg generation_id "$active_generation" '
    .generation_id == $generation_id
    and .flake_shape.active_generation == $generation_id
    and .flake_shape.green_only == true
    and (.flake_shape | has("baseline_red_failures") | not)
    and (.flake_shape | has("missing_system_red") | not)
  ' "$source_file" >/dev/null || reject 'R14 generation or GREEN-only identity is invalid'
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

verify_active_authority_prerequisite() {
  verify_regular_repo_file "$g0_token" 'G0-F2 completion token'
  verify_regular_repo_file "$g0_token_review" 'G0-F2 token-integrity review'
  [[ "$(sha256sum "$g0_token" | awk '{print $1}')" == "$expected_g0_token_sha" ]] || reject 'stale G0-F2 completion token'
  [[ "$(sha256sum "$g0_token_review" | awk '{print $1}')" == "$expected_g0_token_review_sha" ]] || reject 'stale G0-F2 token-integrity review'
  [[ "$(rg -Fxc -- '## Review Result: PASS' "$g0_token_review")" == 1 ]] || reject 'G0-F2 token-integrity review is not PASS'
  jq -e \
    --arg generation_id "$active_generation" \
    --arg plan_path "$authority_plan" \
    --arg plan_sha "$expected_authority_plan_sha" \
    --arg requirements_path "$authority_requirements" \
    --arg requirements_sha "$expected_authority_requirements_sha" '
      .schema_version == "1.2"
      and .generation_id == $generation_id
      and .status == "complete"
      and .owner == "task-13/tf-infra"
      and .prerequisite_id == "task-4b-redis-publish-config-binding"
      and .authority_resolution == "immutable_snapshot_only_at_runtime"
      and .authority_snapshots.runtime_resolution == "snapshot_only"
      and .authority_snapshots.approved_plan.path == $plan_path
      and .authority_snapshots.approved_plan.sha256 == $plan_sha
      and .authority_snapshots.approved_requirements.path == $requirements_path
      and .authority_snapshots.approved_requirements.sha256 == $requirements_sha
      and .authority_snapshots.approved_plan.canonical_source_provenance.runtime_access == "forbidden"
      and .authority_snapshots.approved_requirements.canonical_source_provenance.runtime_access == "forbidden"
      and .parent_token.generation_id == "g0-f1-format-drift-20260905"
      and .parent_token.sha256 == "f178eb60c2c18c27feeea8e0da13a11700dd82372310c426085992547fdd95be"
      and .parent_token.integrity_review.sha256 == "71b33c28ebd337c076065b4442311d977845fd8e0f1f9d225b417c6f526d9e62"
      and .supersedes.active_predecessor_generation_id == "task13-r13-g0-f1-format-drift-20260905"
      and .supersedes.replacement_authority == "R14 immutable plan and requirements snapshots"
      and .supersedes.parent_remains_immutable_historical_evidence == true
      and .supersedes.r11_r12_r13_artifacts_remain_immutable_historical_evidence == true
      and .downstream_exclusions.token_integrity_review_bound == false
      and .downstream_exclusions.token_integrity_review_must_follow_issuance == true
      and .downstream_exclusions.final_r14_implementation_ccr_bound == false
      and .downstream_exclusions.user_receipt_bound == false
      and .downstream_exclusions.shape_or_guarded_flake_result_bound == false
      and .downstream_exclusions.descriptor_or_fv_output_bound == false
      and .immutability_and_scope.runtime_mutable_canonical_access_allowed == false
      and .immutability_and_scope.task_count == 5
      and .immutability_and_scope.task_ids == ["t13-s1", "t13-s2", "t13-s3", "t13-s4", "t13-s5"]
      and all(.issuance_freshness[]; . == true)
    ' "$g0_token" >/dev/null || reject 'G0-F2 completion token contract is invalid'

  printf '%s\n' 'Task-13 G0-F2/R14 immutable authority token is current'
}

verify_authority_snapshots_at_root() {
  local source_root="$1"
  local phase="$2"
  local source_file="$3"
  local plan_file="$source_root/$authority_plan"
  local requirements_file="$source_root/$authority_requirements"
  verify_recorded_source_at_root "$source_root" "$phase" "$expected_authority_plan_sha" "$authority_plan"
  verify_recorded_source_at_root "$source_root" "$phase" "$expected_authority_requirements_sha" "$authority_requirements"
  jq -e 'type == "object"' "$plan_file" >/dev/null || reject "R14 authority plan snapshot is invalid JSON at ${phase}"
  [[ "$(tail -c 1 "$plan_file" | od -An -tuC | tr -d '[:space:]')" == 10 ]] || reject "R14 authority plan snapshot lacks final LF at ${phase}"
  [[ "$(tail -c 1 "$requirements_file" | od -An -tuC | tr -d '[:space:]')" == 10 ]] || reject "R14 authority requirements snapshot lacks final LF at ${phase}"
  jq -e \
    --slurpfile plan "$plan_file" \
    --arg plan_path "$authority_plan" \
    --arg plan_sha "$expected_authority_plan_sha" \
    --arg requirements_path "$authority_requirements" \
    --arg requirements_sha "$expected_authority_requirements_sha" '
      .authority_binding.runtime_resolution == "immutable_snapshot_only"
      and .authority_binding.runtime_mutable_canonical_access == "forbidden"
      and .authority_binding.snapshots.approved_plan == {path:$plan_path, sha256:$plan_sha}
      and .authority_binding.snapshots.approved_requirements == {path:$requirements_path, sha256:$requirements_sha}
      and .authority_binding.active_assertion_contract == $plan[0].assertion_manifests.task13_r14
      and .authority_binding.active_protocol_authority == $plan[0].active_r14_final_protocol_authority
    ' "$source_file" >/dev/null || reject "R14 immutable authority projection is invalid at ${phase}"
}

verify_active_command_source_schema() {
  local source_file="$1"
  jq -e --arg generation_id "$active_generation" '
    .command_source_integrity.generation_id == $generation_id
    and (.command_source_integrity.canonical_order == [
      "scripts/tasks/task-13/r14-shape.just",
      "scripts/tasks/task-13/guarded-just-r14.sh",
      "tests/nix/flake-shape-aarch64-linux-r14.sh",
      "scripts/tasks/task-13/flake-source-snapshot-r14.sh"
    ])
    and ([.command_source_integrity.sources[].path] == .command_source_integrity.canonical_order)
    and (.command_source_integrity.sources | length == 4)
    and (all(.command_source_integrity.sources[];
      (.path | test("^(scripts/tasks/task-13/(r14-shape[.]just|guarded-just-r14[.]sh|flake-source-snapshot-r14[.]sh)|tests/nix/flake-shape-aarch64-linux-r14[.]sh)$"))
      and (.sha256 | test("^[0-9a-f]{64}$"))
    ))
    and (.command_source_integrity.post_validation_mutation_fixture.id == "active-r14-post-validation-live-target-binding-zero-nested-dispatch")
    and (.command_source_integrity.post_validation_mutation_fixture.execution_class == "live_worktree")
    and (.command_source_integrity.post_validation_mutation_fixture.target == "scripts/tasks/task-3b/mod.just")
    and (.command_source_integrity.post_validation_mutation_fixture.recipe == "check")
    and (.command_source_integrity.post_validation_mutation_fixture.mutation_point == "after the live target is copied and before final live target identity/content validation immediately preceding nested dispatch")
    and (.command_source_integrity.post_validation_mutation_fixture.mutations == ["content-mutation", "path-replacement"])
    and (.command_source_integrity.post_validation_mutation_fixture.identity_fields == ["device", "inode", "sha256"])
    and (.command_source_integrity.post_validation_mutation_fixture.copy_directory_rule == "same physical directory as selected live target Justfile")
    and (.command_source_integrity.post_validation_mutation_fixture.relative_working_directory_semantics_preserved == true)
    and (.command_source_integrity.post_validation_mutation_fixture.invocation_target == "verified adjacent per-dispatch copy")
    and (.command_source_integrity.post_validation_mutation_fixture.aba_safety_rule == "final device/inode/SHA validation rejects retained path or content replacement; transient A-B-A cannot change bound copy bytes")
    and (.command_source_integrity.post_validation_mutation_fixture.exact_recipe_selection_preserved == true)
    and (.command_source_integrity.post_validation_mutation_fixture.nested_exit_status_preserved == true)
    and (.command_source_integrity.post_validation_mutation_fixture.cleanup_on == ["success", "nested-failure", "guard-error", "HUP", "INT", "TERM"])
    and (.command_source_integrity.post_validation_mutation_fixture.expected_nested_dispatch_count == 0)
    and (.command_source_integrity.post_validation_mutation_fixture.expected_repository_persistent_write_count == 0)
    and (.command_source_integrity.post_validation_mutation_fixture.expected_exit == 2)
    and (.command_source_integrity.post_validation_mutation_fixture as $fixture | any(.guarded_dispatch.execution_classes.live_worktree[]; .justfile == $fixture.target and .recipe == $fixture.recipe))
    and (.command_source_integrity.reused_target_sources | length == 1)
    and (.command_source_integrity.reused_target_sources[0].path == "scripts/tasks/task-1/mod.just")
    and (.command_source_integrity.reused_target_sources[0].applies_to_recipes == ["flake-local", "flake-linux"])
    and (.command_source_integrity.reused_target_sources[0].sha256 | test("^[0-9a-f]{64}$"))
    and (.guarded_dispatch.execution_classes.immutable_snapshot == [
      {"justfile":"scripts/tasks/task-13/r14-shape.just","recipe":"aarch64-linux-shape-r14-immutable"},
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
  ' "$source_file" >/dev/null || reject 'active R14 command-source integrity schema is missing or incompatible'
}

verify_active_protocol_schema() {
  local source_file="$1"
  jq -e \
    --arg generation_id "$active_generation" \
    --arg token_path "$g0_token" \
    --arg token_sha "$expected_g0_token_sha" \
    --arg review_path "$g0_token_review" \
    --arg review_sha "$expected_g0_token_review_sha" '
      .generation_id == $generation_id
      and .authority_binding.generation_id == $generation_id
      and .authority_binding.token == {
        path:$token_path,
        schema_version:"1.2",
        sha256:$token_sha,
        status:"complete"
      }
      and .authority_binding.token_integrity_review == {
        path:$review_path,
        sha256:$review_sha,
        status:"PASS"
      }
      and .authority_binding.final_implementation_ccr_bound == false
      and .authority_binding.user_receipt_bound == false
      and .authority_binding.shape_or_guarded_flake_result_bound == false
      and .authority_binding.descriptor_or_fv_output_bound == false
      and .final_tree_record_protocol.descriptor_immutable_file_paths == {
        "aarch64_darwin": {
          "first_receipt": ".agents/results/task-13-r14-aarch64-darwin-first-receipt-20260905.json",
          "fv07_replay_receipt": ".agents/results/task-13-r14-aarch64-darwin-fv07-replay-receipt-20260905.json",
          "terminal": ".agents/results/task-13-r14-aarch64-darwin-terminal-20260905.json"
        },
        "aarch64_linux": {
          "first_receipt": ".agents/results/task-13-r14-aarch64-linux-first-receipt-20260905.json",
          "fv07_replay_receipt": ".agents/results/task-13-r14-aarch64-linux-fv07-replay-receipt-20260905.json",
          "terminal": ".agents/results/task-13-r14-aarch64-linux-terminal-20260905.json"
        },
        "x86_64_linux": {
          "first_receipt": ".agents/results/task-13-r14-x86_64-linux-first-receipt-20260905.json",
          "fv07_replay_receipt": ".agents/results/task-13-r14-x86_64-linux-fv07-replay-receipt-20260905.json",
          "terminal": ".agents/results/task-13-r14-x86_64-linux-terminal-20260905.json"
        }
      }
      and .final_tree_record_protocol.fv_input_sets.canonical_order == ["FV01", "FV02", "FV03", "FV04", "FV05", "FV06", "FV07", "FV08"]
      and all(.final_tree_record_protocol.fv_input_sets["FV01", "FV02", "FV03", "FV04", "FV05", "FV06", "FV07", "FV08"];
        .generated_protocol_evidence_input_ids == [
          "gpe-final-tree-repository-manifest",
          "gpe-ignored-input-inventory",
          "gpe-input-coverage-oracle",
          "gpe-source-snapshot-hash-artifact"
        ]
        and (.explicit_ignored_evidence_input_ids | index("ignored-r14-recorded-digest")) != null
      )
      and .operator_surfaces.final_verify.dispatch_by_id["fv-07-supported-system-builder-matrix"].dispatch == [
        "scripts/tasks/task-13/mod.just linux-builder-check <r14-recorded-digest> <gpe-map-digest> fv07-replay",
        "scripts/tasks/task-13/mod.just aarch64-linux-builder-check <r14-recorded-digest> <gpe-map-digest> fv07-replay",
        "scripts/tasks/task-13/mod.just cross-system-verify <r14-recorded-digest> fv07-replay"
      ]
      and .operator_surfaces.final_verify.dispatch_by_id["fv-09-redacted-no-mutation-closeout"].handoff_path == ".agents/results/task-13-r14-final-verify-handoff-20260905.json"
      and .final_tree_snapshot.record_protocol_ref == "assertion_manifests.task13_r14.final_tree_record_protocol"
      and ([.final_tree_snapshot.catalog_fixed_entries[].input_id] | index("g0-f1-approved-plan")) == null
      and ([.final_tree_snapshot.catalog_fixed_entries[].input_id] | index("g0-f1-approved-requirements")) == null
      and ([.final_tree_snapshot.catalog_required_entries[]] | index("g0-f2-authority-plan-snapshot")) != null
      and ([.final_tree_snapshot.catalog_required_entries[]] | index("g0-f2-authority-requirements-snapshot")) != null
      and ([.final_tree_snapshot.catalog_required_entries[]] | index("g0-f2-authority-approval")) != null
      and ([.final_tree_snapshot.catalog_required_entries[]] | index("g0-f2-completion-token")) != null
      and ([.final_tree_snapshot.catalog_required_entries[]] | index("g0-f2-token-integrity-review")) != null
    ' "$source_file" >/dev/null || reject 'active R14 final-tree, descriptor, or authority schema is missing or incompatible'
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
  [[ "$source_root" == /* && -d "$source_root" ]] || reject "active R14 recorded-source root is invalid at ${phase}"
  [[ "$expected_sha" =~ ^[0-9a-f]{64}$ ]] || reject "active R14 recorded-source digest is malformed at ${phase}"
  case "$relative_path" in
    scripts/tasks/task-13/r14-shape.just | scripts/tasks/task-13/guarded-just-r14.sh | tests/nix/flake-shape-aarch64-linux-r14.sh | scripts/tasks/task-13/flake-source-snapshot-r14.sh | scripts/tasks/task-1/mod.just | tests/nix/task-13-r14-authority-plan-20260905.json | tests/nix/task-13-r14-authority-requirements-20260905.md) ;;
    *) reject "active R14 recorded-source path is noncanonical at ${phase}" ;;
  esac
  source_directory="$source_root/$(dirname -- "$relative_path")"
  source_basename="$(basename -- "$relative_path")"
  [[ -d "$source_directory" ]] || reject "active R14 command-source directory is missing at ${phase}"
  physical_directory="$(cd -P -- "$source_directory" && pwd -P)" || reject "active R14 command-source directory cannot be resolved at ${phase}"
  physical_source="${physical_directory}/${source_basename}"
  [[ "$physical_source" == "$source_root/$relative_path" && -f "$physical_source" && ! -L "$physical_source" ]] || reject "active R14 recorded source is missing, linked, or escaped at ${phase}"
  actual_sha="$(sha256sum "$physical_source" | awk '{print $1}')"
  [[ "$actual_sha" == "$expected_sha" ]] || reject "active R14 recorded-source hash mismatch at ${phase}"
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
  expected_sha="$(jq -er '.command_source_integrity.reused_target_sources[0].sha256' "$source_file")" || reject 'active R14 reused Task-1 source digest is missing'
  relative_path="$(jq -er '.command_source_integrity.reused_target_sources[0].path' "$source_file")" || reject 'active R14 reused Task-1 source path is missing'
  verify_recorded_source_at_root "$source_root" "$phase" "$expected_sha" "$relative_path"
}

allowed_pairs_from_snapshot() {
  jq -r '
    .guarded_dispatch.allowed_pairs[]
    | select(
        ((.justfile == "scripts/tasks/task-13/r14-shape.just") or (.justfile | test("^scripts/tasks/[A-Za-z0-9-]+/mod\\.just$")))
        and (.recipe | test("^[A-Za-z0-9][A-Za-z0-9-]*$"))
      )
    | "\(.justfile)\t\(.recipe)"
  ' "$manifest_snapshot" | LC_ALL=C sort -u
}

if [[ "$mode" == 'receipt' ]]; then
  verify_manifest_file "$manifest"
  verify_active_authority_prerequisite
  verify_authority_snapshots_at_root "$repo_root" 'receipt-live' "$manifest"
  verify_active_command_source_schema "$manifest"
  verify_active_protocol_schema "$manifest"
  verify_command_sources_at_root "$repo_root" 'receipt-live' "$manifest"
  printf 'task13_assertion_generation=%s\n' "$active_generation"
  printf 'task13_assertion_manifest_sha256=%s\n' "$snapshot_digest"
  printf 'coordinator record path=%s\n' "$fixed_record"
  printf '%s\n' 'DISPOSITION: ACTIVE_R14_MANIFEST_VERIFIED (zero mutation)'
  exit 0
fi

readonly recorded_digest target_justfile target_recipe
[[ "$recorded_digest" =~ ^[0-9a-f]{64}$ ]] || reject 'recorded digest must be lowercase 64-hex'
[[ "$target_justfile" == 'scripts/tasks/task-13/r14-shape.just' || "$target_justfile" =~ ^scripts/tasks/[A-Za-z0-9-]+/mod\.just$ ]] || reject 'justfile path is not canonical for active R14'
[[ "$target_recipe" =~ ^[A-Za-z0-9][A-Za-z0-9-]*$ ]] || reject 'recipe is not canonical'
verify_regular_repo_file "$target_justfile" 'R14 target Justfile'
manifest_snapshot="$(mktemp "${TMPDIR:-/tmp}/jamye-task13-r14-manifest.XXXXXX")" || reject 'cannot create R14 manifest snapshot'
cp -- "$manifest" "$manifest_snapshot" || reject 'cannot snapshot R14 manifest'
verify_manifest_file "$manifest_snapshot"

verify_inputs_against_snapshot
verify_active_authority_prerequisite
verify_authority_snapshots_at_root "$repo_root" 'live-before-pair-selection' "$manifest_snapshot"
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
verify_active_protocol_schema "$manifest_snapshot"
verify_active_authority_prerequisite
verify_authority_snapshots_at_root "$repo_root" 'live-before-dispatch' "$manifest_snapshot"
active_execution_class="$(active_execution_class_for_pair)" || reject 'active R14 dispatch pair classification failed'
verify_command_sources_at_root "$repo_root" 'live-before-dispatch'

if [[ "$active_execution_class" == 'immutable_snapshot' ]]; then
    if [[ "$target_justfile" == 'scripts/tasks/task-1/mod.just' ]]; then
      verify_reused_task1_source_at_root "$repo_root" 'live-before-snapshot'
    fi

    snapshot_helper_relative='scripts/tasks/task-13/flake-source-snapshot-r14.sh'
    snapshot_helper_expected_sha="$(jq -er --arg path "$snapshot_helper_relative" '.command_source_integrity.sources[] | select(.path == $path) | .sha256' "$manifest_snapshot")" || reject 'active R14 snapshot-helper hash is missing'
    snapshot_helper_copy="$(mktemp "${TMPDIR:-/tmp}/jamye-task13-r14-snapshot-helper.XXXXXX")" || reject 'cannot create verified snapshot-helper copy'
    cp -- "$repo_root/$snapshot_helper_relative" "$snapshot_helper_copy" || reject 'cannot copy the verified snapshot helper'
    [[ "$(sha256sum "$snapshot_helper_copy" | awk '{print $1}')" == "$snapshot_helper_expected_sha" ]] || reject 'verified snapshot-helper copy hash mismatch'

    if ! source_snapshot="$(TASK13_SOURCE_ROOT="$repo_root" bash "$snapshot_helper_copy")"; then
      reject 'filtered source snapshot creation failed'
    fi
    [[ "$source_snapshot" == /nix/store/* && "$source_snapshot" != *$'\n'* && -d "$source_snapshot" ]] || reject 'filtered source snapshot is not a Nix store directory'

    verify_inputs_against_snapshot
    verify_active_authority_prerequisite
    verify_authority_snapshots_at_root "$repo_root" 'live-after-snapshot-creation' "$manifest_snapshot"
    snapshot_manifest="$source_snapshot/$manifest"
    [[ -f "$snapshot_manifest" && ! -L "$snapshot_manifest" ]] || reject 'filtered source snapshot active manifest is missing or linked'
    cmp -s "$manifest_snapshot" "$snapshot_manifest" || reject 'filtered source snapshot active manifest differs from the verified manifest'
    [[ "$(sha256sum "$snapshot_manifest" | awk '{print $1}')" == "$snapshot_digest" ]] || reject 'filtered source snapshot active manifest digest mismatch'
    verify_active_command_source_schema "$snapshot_manifest"
    verify_active_protocol_schema "$snapshot_manifest"
    verify_authority_snapshots_at_root "$source_snapshot" 'immutable-snapshot' "$snapshot_manifest"
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
    verify_active_authority_prerequisite
    verify_authority_snapshots_at_root "$repo_root" 'live-final-pre-dispatch' "$manifest_snapshot"

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

[[ "$active_execution_class" == 'live_worktree' ]] || reject 'active R14 dispatch pair has an invalid execution class'
verify_regular_repo_file "$target_justfile" 'live-target-before-copy'
live_target_identity_before="$(stat -c '%d:%i' -- "$physical_justfile")" || reject 'live target identity cannot be read before copy'
live_target_sha_before="$(sha256sum "$physical_justfile" | awk '{print $1}')"
live_justfile_copy="$(mktemp "$target_directory/.jamye-task13-r14-live-justfile.XXXXXX")" || reject 'cannot create adjacent live target copy'
cp -- "$physical_justfile" "$live_justfile_copy" || reject 'cannot copy live target Justfile'
chmod 0400 -- "$live_justfile_copy" || reject 'cannot make live target copy read-only'
live_copy_directory="$(cd -P -- "$(dirname -- "$live_justfile_copy")" && pwd -P)" || reject 'live target copy directory cannot be resolved'
[[ "$live_copy_directory" == "$physical_directory" && -f "$live_justfile_copy" && ! -L "$live_justfile_copy" ]] || reject 'live target copy does not preserve Justfile directory identity'
live_copy_identity="$(stat -c '%d:%i' -- "$live_justfile_copy")" || reject 'live target copy identity cannot be read'
[[ "$(sha256sum "$live_justfile_copy" | awk '{print $1}')" == "$live_target_sha_before" ]] || reject 'live target copy differs from selected bytes'
verify_command_sources_at_root "$repo_root" 'live-before-worktree-dispatch'
verify_inputs_against_snapshot
verify_active_authority_prerequisite
verify_authority_snapshots_at_root "$repo_root" 'live-before-worktree-dispatch' "$manifest_snapshot"
verify_regular_repo_file "$target_justfile" 'live-target-final-pre-dispatch'
final_live_directory="$(cd -P -- "$target_directory" && pwd -P)" || reject 'live target directory cannot be resolved before dispatch'
final_live_justfile="${final_live_directory}/${target_basename}"
live_target_identity_after="$(stat -c '%d:%i' -- "$final_live_justfile")" || reject 'live target identity cannot be read before dispatch'
[[ "$final_live_justfile" == "$physical_justfile" && "$live_target_identity_after" == "$live_target_identity_before" ]] || reject 'live target path identity changed after binding'
[[ "$(sha256sum "$final_live_justfile" | awk '{print $1}')" == "$live_target_sha_before" ]] || reject 'live target content changed after binding'
[[ -f "$live_justfile_copy" && ! -L "$live_justfile_copy" && "$(stat -c '%d:%i' -- "$live_justfile_copy")" == "$live_copy_identity" && "$(sha256sum "$live_justfile_copy" | awk '{print $1}')" == "$live_target_sha_before" ]] || reject 'bound live target copy changed before dispatch'
printf 'task13_assertion_manifest_sha256=%s\n' "$snapshot_digest"
printf 'task13_assertion_generation=%s\n' "$generation_id"
rm -f -- "$manifest_snapshot"
manifest_snapshot=''
export TASK13_ASSERTION_GENERATION="$generation_id"
export TASK13_ASSERTION_DIGEST="$snapshot_digest"
if just --justfile "$live_justfile_copy" "$target_recipe"; then
  nested_exit=0
else
  nested_exit=$?
fi
rm -f -- "$live_justfile_copy"
live_justfile_copy=''
exit "$nested_exit"
