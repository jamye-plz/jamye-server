#!/usr/bin/env bash
set -euo pipefail
readonly active_generation='task13-r19-g0-f7-guard-argument-consistency-20260906'
readonly manifest='tests/nix/task-13-assertion-manifest-r19-g0-f7.json'
readonly lock='tests/nix/task-13-assertion-manifest-r19-g0-f7.sha256'
readonly fixed_record='.agents/results/task-13-s2-r19-g0-f7-manifest-digest-20260906.txt'
readonly g0_token='.agents/results/task-4b-redis-publish-completion-g0-f7-r19-20260906.json'
readonly expected_g0_token_sha='982a9d0bd606f9b811ebdce3ed09356663ce924eb3c61b2ceaddd6c0fc8aa46b'
readonly g0_token_review='.agents/results/review-task13-g0-f7-r19-token-integrity-r1-20260907.md'
readonly expected_g0_token_review_sha='d3f62441a7e59d9d90a7ed608fe72a4b293ff8ec0ea3fd6c5e3e831fa1cd06ff'
readonly authority_plan='tests/nix/task-13-r19-authority-plan-20260906.json'
readonly expected_authority_plan_sha='3ea7333086d4d23a1bf4350397babd54c971aafb14dd08eb38386e0ea557b334'
readonly authority_requirements='tests/nix/task-13-r19-authority-requirements-20260906.md'
readonly expected_authority_requirements_sha='61065811a1e314edcd1c711999279168e7b31cd87f5c72b64f24dea58015afc3'
readonly fixture_path='tests/nix/coverage-executor-toctou-r19.sh'
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
  printf '%s\n' 'usage: guarded-just-r19.sh task13-r19-g0-f7-guard-argument-consistency-20260906 (--receipt | <recorded-digest> <justfile-relative-path> <recipe>)' >&2
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
[[ "$(pwd -P)" == "$repo_root" ]] || reject 'R19 guard must start at the repository root'
[[ "$generation_id" == "$active_generation" ]] || reject 'unknown Task-13 R19 assertion generation'
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

verify_bound_live_file_and_copy() {
  local relative_path="$1"
  local expected_physical_path="$2"
  local expected_source_identity="$3"
  local expected_sha="$4"
  local copy_path="$5"
  local expected_copy_identity="$6"
  local phase="$7"
  local directory basename physical_directory physical_path
  verify_regular_repo_file "$relative_path" "$phase"
  directory="$repo_root/$(dirname -- "$relative_path")"
  basename="$(basename -- "$relative_path")"
  physical_directory="$(cd -P -- "$directory" && pwd -P)" || reject "bound source directory cannot be resolved at ${phase}"
  physical_path="${physical_directory}/${basename}"
  [[ "$physical_path" == "$expected_physical_path" ]] || reject "bound source path changed at ${phase}"
  [[ "$(stat -c '%d:%i' -- "$physical_path")" == "$expected_source_identity" ]] || reject "bound source identity changed at ${phase}"
  [[ "$(sha256sum "$physical_path" | awk '{print $1}')" == "$expected_sha" ]] || reject "bound source content changed at ${phase}"
  [[ "$copy_path" == /* && -f "$copy_path" && ! -L "$copy_path" ]] || reject "bound read-only copy is missing or linked at ${phase}"
  [[ "$(stat -c '%a' -- "$copy_path")" == 400 ]] || reject "bound copy is not read-only at ${phase}"
  [[ "$(stat -c '%d:%i' -- "$copy_path")" == "$expected_copy_identity" ]] || reject "bound read-only copy identity changed at ${phase}"
  [[ "$(sha256sum "$copy_path" | awk '{print $1}')" == "$expected_sha" ]] || reject "bound read-only copy content changed at ${phase}"
}

wait_at_deterministic_post_copy_fixture_barrier() {
  local fixture_mode="${TASK13_R19_POST_COPY_FIXTURE_MODE:-}"
  local fixture_root="${TASK13_R19_FIXTURE_REPO_ROOT:-}"
  local ready_fd="${TASK13_R19_FIXTURE_READY_FD:-}"
  local release_fd="${TASK13_R19_FIXTURE_RELEASE_FD:-}"
  local temp_root release
  if [[ -z "$fixture_mode$fixture_root$ready_fd$release_fd" ]]; then
    return 0
  fi
  [[ "$target_justfile" == 'scripts/tasks/task-13/mod.just' && "$target_recipe" == 'coverage-r19-exec' ]] || reject 'R19 post-copy fixture is restricted to the private coverage GREEN pair'
  [[ "$fixture_mode" == 'content-mutation' || "$fixture_mode" == 'path-replacement' ]] || reject 'R19 post-copy fixture mode is invalid'
  [[ "$fixture_root" == "$repo_root" ]] || reject 'R19 post-copy fixture root does not match the guarded repository'
  temp_root="$(cd -P -- "${TMPDIR:-/tmp}" && pwd -P)" || reject 'R19 post-copy fixture temp root cannot be resolved'
  case "$repo_root" in
    "$temp_root"/jamye-task13-r19-toctou.*/repo) ;;
    *) reject 'R19 post-copy fixture is forbidden outside its disposable repository' ;;
  esac
  [[ "$ready_fd" =~ ^[0-9]+$ && "$release_fd" =~ ^[0-9]+$ && "$ready_fd" != "$release_fd" ]] || reject 'R19 post-copy fixture descriptors are invalid'
  printf 'ready:%s\n' "$fixture_mode" >&"$ready_fd" || reject 'R19 post-copy fixture ready signal failed'
  IFS= read -r release <&"$release_fd" || reject 'R19 post-copy fixture release signal failed'
  [[ "$release" == "release:${fixture_mode}" ]] || reject 'R19 post-copy fixture release token is invalid'
}
for required_path in "$manifest" "$lock" "$fixed_record"; do
  verify_regular_repo_file "$required_path" 'R19 manifest material'
done
verify_manifest_file() {
  local source_file="$1"
  local expected
  cmp -s <(jq -cS '.' "$source_file") "$source_file" || reject 'R19 manifest bytes are not canonical JSON plus LF'
  snapshot_digest="$(sha256sum "$source_file" | awk '{print $1}')"
  [[ "$snapshot_digest" =~ ^[0-9a-f]{64}$ ]] || reject 'R19 manifest digest is malformed'
  expected="${snapshot_digest}"$'\n'
  cmp -s <(printf '%s' "$expected") "$lock" || reject 'R19 manifest lock is malformed or mismatched'
  cmp -s <(printf '%s' "$expected") "$fixed_record" || reject 'R19 coordinator record is malformed or mismatched'
  jq -e --arg generation_id "$active_generation" '
    .generation_id == $generation_id
    and .flake_shape.active_generation == $generation_id
    and .flake_shape.green_only == true
    and (.flake_shape | has("baseline_red_failures") | not)
    and (.flake_shape | has("missing_system_red") | not)
  ' "$source_file" >/dev/null || reject 'R19 generation or GREEN-only identity is invalid'
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
  verify_regular_repo_file "$g0_token" 'G0-F7 completion token'
  verify_regular_repo_file "$g0_token_review" 'G0-F7 token-integrity review'
  [[ "$(sha256sum "$g0_token" | awk '{print $1}')" == "$expected_g0_token_sha" ]] || reject 'stale G0-F7 completion token'
  [[ "$(sha256sum "$g0_token_review" | awk '{print $1}')" == "$expected_g0_token_review_sha" ]] || reject 'stale G0-F7 token-integrity review'
  [[ "$(rg -Fxc -- '## Review Result: PASS' "$g0_token_review")" == 1 ]] || reject 'G0-F7 token-integrity review is not PASS'
  jq -e \
    --slurpfile plan "$authority_plan" \
    --arg generation_id "$active_generation" \
    --arg plan_path "$authority_plan" \
    --arg plan_sha "$expected_authority_plan_sha" \
    --arg requirements_path "$authority_requirements" \
    --arg requirements_sha "$expected_authority_requirements_sha" '
      .schema_version == "1.7"
      and .generation_id == $generation_id
      and .status == "complete"
      and .owner == "task-13/tf-infra"
      and .prerequisite_id == "task-4b-redis-publish-config-binding"
      and .authority_resolution == "immutable_snapshot_only_at_runtime"
      and .authority_snapshots.runtime_resolution == "snapshot_only"
      and .authority_snapshots.approved_plan.path == $plan_path
      and .authority_snapshots.approved_plan.sha256 == $plan_sha
      and .authority_snapshots.approved_plan.activated_as_immutable_by_this_token == true
      and .authority_snapshots.approved_plan.candidate_status_before_token == "non_authoritative"
      and .authority_snapshots.approved_plan.canonical_json_plus_lf_at_issuance == true
      and .authority_snapshots.approved_plan.ends_with_lf == true
      and .authority_snapshots.approved_plan.regular_file_at_issuance == true
      and .authority_snapshots.approved_plan.reviewed_by_approval_evidence == true
      and .authority_snapshots.approved_plan.symlink_at_issuance == false
      and .authority_snapshots.approved_plan.valid_json_object_at_issuance == true
      and .authority_snapshots.approved_requirements.path == $requirements_path
      and .authority_snapshots.approved_requirements.sha256 == $requirements_sha
      and .authority_snapshots.approved_requirements.activated_as_immutable_by_this_token == true
      and .authority_snapshots.approved_requirements.candidate_status_before_token == "non_authoritative"
      and .authority_snapshots.approved_requirements.ends_with_lf == true
      and .authority_snapshots.approved_requirements.regular_file_at_issuance == true
      and .authority_snapshots.approved_requirements.reviewed_by_approval_evidence == true
      and .authority_snapshots.approved_requirements.symlink_at_issuance == false
      and (.authority_snapshots.runtime_rule | test("R19") and test("R11-R18"))
      and .approval_evidence.path == ".agents/results/task-13-g0-f7-r19-approval-20260906.md"
      and .approval_evidence.sha256 == "6d1161917e0800d4e4e2622c37f1c69b2f5ab71774508aee80e3a68d57c85c33"
      and .approval_evidence.status == "APPROVED_PRE_TOKEN"
      and .approval_evidence.approved_snapshot_sha256 == {plan:$plan_sha, requirements:$requirements_sha}
      and ([.approval_evidence.approved_plan_reviews[] | .path] == $plan[0].assertion_manifests.task13_r19.reviews.plan)
      and ([.approval_evidence.approved_plan_reviews[] | .kind] == ["completeness", "meta", "simplicity"])
      and ([.approval_evidence.approved_plan_reviews[] | .status] == ["PASS", "PASS", "PASS"])
      and ([.approval_evidence.approved_plan_reviews[] | .sha256] | all(test("^[0-9a-f]{64}$")))
      and .approval_evidence.ordered_review_paths_equal_final_plan == true
      and .parent_token.runtime_authority_under_r19 == false
      and .parent_token.r18_manifest_lock_or_record_bound == false
      and .supersedes.active_predecessor_generation_id == "task13-r18-g0-f6-review-binding-correction-20260906"
      and .supersedes.replacement_authority == "R19 immutable plan and requirements snapshots"
      and .supersedes.historical_r18_guard_argument_defect_confirmed == true
      and .supersedes.coverage_command_argv_superseded == false
      and .supersedes.only_guard_argument_consistency_authority_successor == true
      and .downstream_exclusions.current_generation_token_integrity_review_bound == false
      and .downstream_exclusions.current_generation_token_integrity_review_must_follow_issuance == true
      and .downstream_exclusions.r19_frozen_source_guard_or_fixture_bound == false
      and .downstream_exclusions.r19_manifest_lock_or_record_bound == false
      and .downstream_exclusions.final_r19_implementation_ccr_bound == false
      and .downstream_exclusions.user_receipt_bound == false
      and .downstream_exclusions.coverage_output_bound == false
      and .downstream_exclusions.shape_or_guarded_flake_result_bound == false
      and .downstream_exclusions.descriptor_or_fv_output_bound == false
      and .downstream_exclusions.final_handoff_bound == false
      and .downstream_exclusions.self_hash_bound == false
      and .immutability_and_scope.runtime_mutable_canonical_access_allowed == false
      and .immutability_and_scope.generation_argument_consistency_correction_only == true
      and .immutability_and_scope.coverage_commands_byte_identical_to_r18_and_r17 == true
      and .immutability_and_scope.r18_relocked_reinterpreted_or_dispatched == false
      and .immutability_and_scope.task_count == 5
      and .immutability_and_scope.task_ids == ["t13-s1", "t13-s2", "t13-s3", "t13-s4", "t13-s5"]
      and all(.issuance_freshness[]; . == true)
      and .generation_argument_consistency.assertion_generation == $generation_id
      and .generation_argument_consistency.token_generation == $generation_id
      and .generation_argument_consistency.guarded_dispatch_first_token == $generation_id
      and .generation_argument_consistency.active_successor_generation == $generation_id
      and .generation_argument_consistency.approval_projection_generation == $generation_id
      and .generation_argument_consistency.verified_equal_at_issuance == true
    ' "$g0_token" >/dev/null || reject 'G0-F7 completion token contract is invalid'

  printf '%s\n' 'Task-13 G0-F7/R19 immutable authority token is current'
}

verify_authority_snapshots_at_root() {
  local source_root="$1"
  local phase="$2"
  local source_file="$3"
  local plan_file="$source_root/$authority_plan"
  local requirements_file="$source_root/$authority_requirements"
  verify_recorded_source_at_root "$source_root" "$phase" "$expected_authority_plan_sha" "$authority_plan"
  verify_recorded_source_at_root "$source_root" "$phase" "$expected_authority_requirements_sha" "$authority_requirements"
  jq -e 'type == "object"' "$plan_file" >/dev/null || reject "R19 authority plan snapshot is invalid JSON at ${phase}"
  [[ "$(tail -c 1 "$plan_file" | od -An -tuC | tr -d '[:space:]')" == 10 ]] || reject "R19 authority plan snapshot lacks final LF at ${phase}"
  [[ "$(tail -c 1 "$requirements_file" | od -An -tuC | tr -d '[:space:]')" == 10 ]] || reject "R19 authority requirements snapshot lacks final LF at ${phase}"
  jq -e \
    --slurpfile plan "$plan_file" \
    --arg generation_id "$active_generation" \
    --arg plan_path "$authority_plan" \
    --arg plan_sha "$expected_authority_plan_sha" \
    --arg requirements_path "$authority_requirements" \
    --arg requirements_sha "$expected_authority_requirements_sha" '
      $plan[0].assertion_manifests.task13_r19.generation == $generation_id
      and $plan[0].assertion_manifests.task13_r19.generation == $plan[0].assertion_manifests.task13_r19.generation_argument_invariant.assertion_generation
      and ($plan[0].assertion_manifests.task13_r19.guarded_dispatch.arguments | split(" ")[0]) == $plan[0].assertion_manifests.task13_r19.generation
      and $plan[0].assertion_manifests.task13_r19.generation == "task13-r19-g0-f7-guard-argument-consistency-20260906"
      and .authority_binding.runtime_resolution == "immutable_snapshot_only"
      and .authority_binding.runtime_mutable_canonical_access == "forbidden"
      and .authority_binding.snapshots.approved_plan == {path:$plan_path, sha256:$plan_sha}
      and .authority_binding.snapshots.approved_requirements == {path:$requirements_path, sha256:$requirements_sha}
      and .authority_binding.active_assertion_contract == $plan[0].assertion_manifests.task13_r19
      and .authority_binding.active_protocol_authority == $plan[0].active_r19_final_protocol_authority
    ' "$source_file" >/dev/null || reject "R19 immutable authority projection is invalid at ${phase}"
}

verify_active_command_source_schema() {
  local source_file="$1"
  jq -e --arg generation_id "$active_generation" '
    .command_source_integrity.generation_id == $generation_id
    and (.command_source_integrity.canonical_order == [
      "scripts/tasks/task-13/r19-shape.just",
      "scripts/tasks/task-13/guarded-just-r19.sh",
      "tests/nix/flake-shape-aarch64-linux-r19.sh",
      "scripts/tasks/task-13/flake-source-snapshot-r19.sh"
    ])
    and ([.command_source_integrity.sources[].path] == .command_source_integrity.canonical_order)
    and (.command_source_integrity.sources | length == 4)
    and (all(.command_source_integrity.sources[];
      (.path | test("^(scripts/tasks/task-13/(r19-shape[.]just|guarded-just-r19[.]sh|flake-source-snapshot-r19[.]sh)|tests/nix/flake-shape-aarch64-linux-r19[.]sh)$"))
      and (.sha256 | test("^[0-9a-f]{64}$"))
    ))
    and .command_source_integrity.live_auxiliary_dependencies == []
    and (.command_source_integrity.post_validation_mutation_fixture.id == "active-r19-post-copy-coverage-executor-mutation-zero-nested-dispatch")
    and (.command_source_integrity.post_validation_mutation_fixture.fixture_kind == "executable-production-barrier")
    and (.command_source_integrity.post_validation_mutation_fixture.executable_path == "tests/nix/coverage-executor-toctou-r19.sh")
    and (.command_source_integrity.post_validation_mutation_fixture.executable_sha256 | test("^[0-9a-f]{64}$"))
    and (.command_source_integrity.post_validation_mutation_fixture.public_transport == "just --justfile scripts/tasks/task-13/mod.just coverage-r19 <r19-recorded-digest>")
    and (.command_source_integrity.post_validation_mutation_fixture.barrier_kind == "fixed-file-descriptor-ready-release")
    and (.command_source_integrity.post_validation_mutation_fixture.production_validation_primitive == "verify_bound_live_file_and_copy")
    and (.command_source_integrity.post_validation_mutation_fixture.disposable_repository_only == true)
    and (.command_source_integrity.post_validation_mutation_fixture.real_repository_mutation_count == 0)
    and (.command_source_integrity.post_validation_mutation_fixture.execution_class == "live_worktree")
    and (.command_source_integrity.post_validation_mutation_fixture.target == "scripts/tasks/task-13/mod.just")
    and (.command_source_integrity.post_validation_mutation_fixture.recipe == "coverage-r19-exec")
    and (.command_source_integrity.post_validation_mutation_fixture.coverage_executor_recipes == ["coverage-r19-exec"])
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
    and ([.guarded_dispatch.execution_classes.live_worktree[]
      | select(.justfile == "scripts/tasks/task-13/mod.just")
      | .recipe] == ["coverage-r19-exec"])
    and (.command_source_integrity.reused_target_sources | length == 1)
    and (.command_source_integrity.reused_target_sources[0].path == "scripts/tasks/task-1/mod.just")
    and (.command_source_integrity.reused_target_sources[0].applies_to_recipes == ["flake-local", "flake-linux"])
    and (.command_source_integrity.reused_target_sources[0].sha256 | test("^[0-9a-f]{64}$"))
    and (.guarded_dispatch.execution_classes.immutable_snapshot == [
      {"justfile":"scripts/tasks/task-13/r19-shape.just","recipe":"aarch64-linux-shape-r19-immutable"},
      {"justfile":"scripts/tasks/task-1/mod.just","recipe":"flake-linux"},
      {"justfile":"scripts/tasks/task-1/mod.just","recipe":"flake-local"}
    ])
    and (.guarded_dispatch.execution_classes.live_worktree | length == 16)
    and (.guarded_dispatch.allowed_pairs | length == 19)
    and ((.guarded_dispatch.execution_classes.immutable_snapshot + .guarded_dispatch.execution_classes.live_worktree) as $classified
      | ($classified | length) == (.guarded_dispatch.allowed_pairs | length)
      and ([$classified[] | "\(.justfile)\u0000\(.recipe)"] | sort) == ([.guarded_dispatch.allowed_pairs[] | "\(.justfile)\u0000\(.recipe)"] | sort)
      and ([$classified[] | "\(.justfile)\u0000\(.recipe)"] | unique | length) == ($classified | length)
    )
  ' "$source_file" >/dev/null || reject 'active R19 command-source integrity schema is missing or incompatible'
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
      and .authority_binding.runtime_resolution == "immutable_snapshot_only"
      and .authority_binding.runtime_mutable_canonical_access == "forbidden"
      and .authority_binding.token == {
        path:$token_path,
        schema_version:"1.7",
        sha256:$token_sha,
        status:"complete"
      }
      and .authority_binding.token_integrity_review == {
        path:$review_path,
        sha256:$review_sha,
        status:"PASS"
      }
      and .authority_binding.active_assertion_contract.generation == $generation_id
      and .authority_binding.active_assertion_contract.authority_digest == "<r19-recorded-digest>"
      and (.authority_binding.active_assertion_contract.coverage.green.ordered_commands | type == "array" and length == 4 and all(.[]; type == "string" and length > 0))
      and .authority_binding.active_assertion_contract.coverage.green.shared_target_directory == "target/task-13-llvm-cov"
      and .authority_binding.active_assertion_contract.coverage.fv03_binding == "FV03 retains its fifth public slot exactly as scripts/tasks/task-13/mod.just coverage-r19 <r19-recorded-digest>; that transport selects only coverage-r19-exec and consumes this R19-owned serialized collection contract."
      and .coverage == .authority_binding.active_assertion_contract.coverage
      and ((.authority_binding.active_assertion_contract.guarded_dispatch.allowed_pairs | map(split("::") | {justfile:.[0], recipe:.[1]})) as $projection_pairs
        | ([$projection_pairs[] | "\(.justfile)\u0000\(.recipe)"] | sort) == ([.guarded_dispatch.allowed_pairs[] | "\(.justfile)\u0000\(.recipe)"] | sort))
      and ([.guarded_dispatch.allowed_pairs[] | select(.recipe == "coverage-r19-exec")] | length == 1)
      and ([.guarded_dispatch.allowed_pairs[] | select(.recipe | test("^coverage(-r[0-9]+)?$|^coverage-red"))] | length == 0)
      and .authority_binding.final_implementation_ccr_bound == false
      and .authority_binding.user_receipt_bound == false
      and .authority_binding.coverage_output_bound == false
      and .authority_binding.shape_or_guarded_flake_result_bound == false
      and .authority_binding.descriptor_or_fv_output_bound == false
      and .final_tree_record_protocol.descriptor_immutable_file_paths == {
        "aarch64_darwin": {
          "first_receipt": ".agents/results/task-13-r19-aarch64-darwin-first-receipt-20260906.json",
          "fv07_replay_receipt": ".agents/results/task-13-r19-aarch64-darwin-fv07-replay-receipt-20260906.json",
          "terminal": ".agents/results/task-13-r19-aarch64-darwin-terminal-20260906.json"
        },
        "aarch64_linux": {
          "first_receipt": ".agents/results/task-13-r19-aarch64-linux-first-receipt-20260906.json",
          "fv07_replay_receipt": ".agents/results/task-13-r19-aarch64-linux-fv07-replay-receipt-20260906.json",
          "terminal": ".agents/results/task-13-r19-aarch64-linux-terminal-20260906.json"
        },
        "x86_64_linux": {
          "first_receipt": ".agents/results/task-13-r19-x86_64-linux-first-receipt-20260906.json",
          "fv07_replay_receipt": ".agents/results/task-13-r19-x86_64-linux-fv07-replay-receipt-20260906.json",
          "terminal": ".agents/results/task-13-r19-x86_64-linux-terminal-20260906.json"
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
        and (.explicit_ignored_evidence_input_ids | index("ignored-r19-recorded-digest")) != null
      )
      and .operator_surfaces.final_verify.dispatch_by_id["fv-07-supported-system-builder-matrix"].dispatch == [
        "scripts/tasks/task-13/mod.just linux-builder-check <r19-recorded-digest> <gpe-map-digest> fv07-replay",
        "scripts/tasks/task-13/mod.just aarch64-linux-builder-check <r19-recorded-digest> <gpe-map-digest> fv07-replay",
        "scripts/tasks/task-13/mod.just cross-system-verify <r19-recorded-digest> fv07-replay"
      ]
      and .operator_surfaces.final_verify.dispatch_by_id["fv-09-redacted-no-mutation-closeout"].handoff_path == ".agents/results/task-13-r19-final-verify-handoff-20260906.json"
      and .final_tree_snapshot.record_protocol_ref == "assertion_manifests.task13_r19.final_tree_record_protocol"
      and ([.final_tree_snapshot.catalog_fixed_entries[].input_id] | index("g0-f1-approved-plan")) == null
      and ([.final_tree_snapshot.catalog_fixed_entries[].input_id] | index("g0-f1-approved-requirements")) == null
      and ([.final_tree_snapshot.catalog_required_entries[]] | index("g0-f7-authority-plan-snapshot")) != null
      and ([.final_tree_snapshot.catalog_required_entries[]] | index("g0-f7-authority-requirements-snapshot")) != null
      and ([.final_tree_snapshot.catalog_required_entries[]] | index("g0-f7-authority-approval")) != null
      and ([.final_tree_snapshot.catalog_required_entries[]] | index("g0-f7-completion-token")) != null
      and ([.final_tree_snapshot.catalog_required_entries[]] | index("g0-f7-token-integrity-review")) != null
    ' "$source_file" >/dev/null || reject 'active R19 final-tree, descriptor, or authority schema is missing or incompatible'
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
  [[ "$source_root" == /* && -d "$source_root" ]] || reject "active R19 recorded-source root is invalid at ${phase}"
  [[ "$expected_sha" =~ ^[0-9a-f]{64}$ ]] || reject "active R19 recorded-source digest is malformed at ${phase}"
  case "$relative_path" in
    scripts/tasks/task-13/r19-shape.just | scripts/tasks/task-13/guarded-just-r19.sh | tests/nix/flake-shape-aarch64-linux-r19.sh | scripts/tasks/task-13/flake-source-snapshot-r19.sh | tests/nix/coverage-executor-toctou-r19.sh | scripts/tasks/task-1/mod.just | tests/nix/task-13-r19-authority-plan-20260906.json | tests/nix/task-13-r19-authority-requirements-20260906.md) ;;
    *) reject "active R19 recorded-source path is noncanonical at ${phase}" ;;
  esac
  source_directory="$source_root/$(dirname -- "$relative_path")"
  source_basename="$(basename -- "$relative_path")"
  [[ -d "$source_directory" ]] || reject "active R19 command-source directory is missing at ${phase}"
  physical_directory="$(cd -P -- "$source_directory" && pwd -P)" || reject "active R19 command-source directory cannot be resolved at ${phase}"
  physical_source="${physical_directory}/${source_basename}"
  [[ "$physical_source" == "$source_root/$relative_path" && -f "$physical_source" && ! -L "$physical_source" ]] || reject "active R19 recorded source is missing, linked, or escaped at ${phase}"
  actual_sha="$(sha256sum "$physical_source" | awk '{print $1}')"
  [[ "$actual_sha" == "$expected_sha" ]] || reject "active R19 recorded-source hash mismatch at ${phase}"
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

verify_fixture_at_root() {
  local source_root="$1"
  local phase="$2"
  local source_file="$3"
  local expected_sha
  expected_sha="$(jq -er '.command_source_integrity.post_validation_mutation_fixture.executable_sha256' "$source_file")" || reject "R19 fixture digest is missing at ${phase}"
  [[ "$expected_sha" =~ ^[0-9a-f]{64}$ ]] || reject "R19 fixture digest is malformed at ${phase}"
  verify_recorded_source_at_root "$source_root" "$phase" "$expected_sha" "$fixture_path"
}

verify_reused_task1_source_at_root() {
  local source_root="$1"
  local phase="$2"
  local source_file="${3:-$manifest_snapshot}"
  local expected_sha relative_path
  expected_sha="$(jq -er '.command_source_integrity.reused_target_sources[0].sha256' "$source_file")" || reject 'active R19 reused Task-1 source digest is missing'
  relative_path="$(jq -er '.command_source_integrity.reused_target_sources[0].path' "$source_file")" || reject 'active R19 reused Task-1 source path is missing'
  verify_recorded_source_at_root "$source_root" "$phase" "$expected_sha" "$relative_path"
}

allowed_pairs_from_snapshot() {
  jq -r '
    .guarded_dispatch.allowed_pairs[]
    | select(
        ((.justfile == "scripts/tasks/task-13/r19-shape.just") or (.justfile | test("^scripts/tasks/[A-Za-z0-9-]+/mod\\.just$")))
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
  verify_fixture_at_root "$repo_root" 'receipt-live' "$manifest"
  printf 'task13_assertion_generation=%s\n' "$active_generation"
  printf 'task13_assertion_manifest_sha256=%s\n' "$snapshot_digest"
  printf 'coordinator record path=%s\n' "$fixed_record"
  printf '%s\n' 'DISPOSITION: ACTIVE_R19_MANIFEST_VERIFIED (zero mutation)'
  exit 0
fi

readonly recorded_digest target_justfile target_recipe
[[ "$recorded_digest" =~ ^[0-9a-f]{64}$ ]] || reject 'recorded digest must be lowercase 64-hex'
[[ "$target_justfile" == 'scripts/tasks/task-13/r19-shape.just' || "$target_justfile" =~ ^scripts/tasks/[A-Za-z0-9-]+/mod\.just$ ]] || reject 'justfile path is not canonical for active R19'
[[ "$target_recipe" =~ ^[A-Za-z0-9][A-Za-z0-9-]*$ ]] || reject 'recipe is not canonical'
verify_regular_repo_file "$target_justfile" 'R19 target Justfile'
manifest_snapshot="$(mktemp "${TMPDIR:-/tmp}/jamye-task13-r19-manifest.XXXXXX")" || reject 'cannot create R19 manifest snapshot'
cp -- "$manifest" "$manifest_snapshot" || reject 'cannot snapshot R19 manifest'
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
active_execution_class="$(active_execution_class_for_pair)" || reject 'active R19 dispatch pair classification failed'
verify_command_sources_at_root "$repo_root" 'live-before-dispatch'
verify_fixture_at_root "$repo_root" 'live-before-dispatch' "$manifest_snapshot"

if [[ "$active_execution_class" == 'immutable_snapshot' ]]; then
    if [[ "$target_justfile" == 'scripts/tasks/task-1/mod.just' ]]; then
      verify_reused_task1_source_at_root "$repo_root" 'live-before-snapshot'
    fi

    snapshot_helper_relative='scripts/tasks/task-13/flake-source-snapshot-r19.sh'
    snapshot_helper_expected_sha="$(jq -er --arg path "$snapshot_helper_relative" '.command_source_integrity.sources[] | select(.path == $path) | .sha256' "$manifest_snapshot")" || reject 'active R19 snapshot-helper hash is missing'
    snapshot_helper_copy="$(mktemp "${TMPDIR:-/tmp}/jamye-task13-r19-snapshot-helper.XXXXXX")" || reject 'cannot create verified snapshot-helper copy'
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
    verify_fixture_at_root "$source_snapshot" 'immutable-snapshot' "$snapshot_manifest"
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

[[ "$active_execution_class" == 'live_worktree' ]] || reject 'active R19 dispatch pair has an invalid execution class'
verify_regular_repo_file "$target_justfile" 'live-target-before-copy'
live_target_identity_before="$(stat -c '%d:%i' -- "$physical_justfile")" || reject 'live target identity cannot be read before copy'
live_target_sha_before="$(sha256sum "$physical_justfile" | awk '{print $1}')"
live_justfile_copy="$(mktemp "$target_directory/.jamye-task13-r19-live-justfile.XXXXXX")" || reject 'cannot create adjacent live target copy'
cp -- "$physical_justfile" "$live_justfile_copy" || reject 'cannot copy live target Justfile'
chmod 0400 -- "$live_justfile_copy" || reject 'cannot make live target copy read-only'
live_copy_directory="$(cd -P -- "$(dirname -- "$live_justfile_copy")" && pwd -P)" || reject 'live target copy directory cannot be resolved'
live_justfile_copy="${live_copy_directory}/$(basename -- "$live_justfile_copy")"
[[ "$live_copy_directory" == "$physical_directory" && -f "$live_justfile_copy" && ! -L "$live_justfile_copy" ]] || reject 'live target copy does not preserve Justfile directory identity'
live_copy_identity="$(stat -c '%d:%i' -- "$live_justfile_copy")" || reject 'live target copy identity cannot be read'
[[ "$(sha256sum "$live_justfile_copy" | awk '{print $1}')" == "$live_target_sha_before" ]] || reject 'live target copy differs from selected bytes'

verify_command_sources_at_root "$repo_root" 'live-before-worktree-dispatch'
verify_inputs_against_snapshot
verify_active_authority_prerequisite
verify_authority_snapshots_at_root "$repo_root" 'live-before-worktree-dispatch' "$manifest_snapshot"
verify_fixture_at_root "$repo_root" 'live-before-worktree-dispatch' "$manifest_snapshot"
wait_at_deterministic_post_copy_fixture_barrier
verify_fixture_at_root "$repo_root" 'live-final-pre-dispatch' "$manifest_snapshot"
verify_bound_live_file_and_copy "$target_justfile" "$physical_justfile" "$live_target_identity_before" "$live_target_sha_before" "$live_justfile_copy" "$live_copy_identity" 'live-target-final-pre-dispatch'
printf 'task13_assertion_manifest_sha256=%s\n' "$snapshot_digest"
printf 'task13_assertion_generation=%s\n' "$generation_id"
rm -f -- "$manifest_snapshot"
manifest_snapshot=''
export TASK13_ASSERTION_GENERATION="$generation_id"
export TASK13_ASSERTION_DIGEST="$snapshot_digest"
export TASK13_BOUND_JUSTFILE="$live_justfile_copy"
export TASK13_BOUND_JUSTFILE_IDENTITY="$live_copy_identity"
export TASK13_BOUND_JUSTFILE_SHA256="$live_target_sha_before"
unset TASK13_COVERAGE_RED_ASSERTION TASK13_COVERAGE_RED_ASSERTION_IDENTITY TASK13_COVERAGE_RED_ASSERTION_SHA256
if just --justfile "$live_justfile_copy" "$target_recipe"; then
  nested_exit=0
else
  nested_exit=$?
fi
rm -f -- "$live_justfile_copy"
live_justfile_copy=''
exit "$nested_exit"
