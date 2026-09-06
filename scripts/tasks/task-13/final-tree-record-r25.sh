#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

# Internal-only Task-13 R25 final-tree, generated-proof-evidence, and package
# descriptor owner.  A copied guarded recipe is the only supported caller.

readonly GENERATION='task13-r25-g0-f13-trust-root-cross-binding-20260907'
readonly MANIFEST_REL='tests/nix/task-13-assertion-manifest-r25-g0-f13.json'
readonly LOCK_REL='tests/nix/task-13-assertion-manifest-r25-g0-f13.sha256'
readonly RECORD_REL='.agents/results/task-13-s2-r25-g0-f13-manifest-digest-20260907.txt'
readonly CATALOG_REL='tests/nix/task-13-final-input-catalog.json'
readonly SELF_REL='scripts/tasks/task-13/final-tree-record-r25.sh'
readonly SHAPE_REL='scripts/tasks/task-13/r25-shape.just'
readonly STRICT_REL='scripts/tasks/task-13/strict-entrypoint-r25.sh'

readonly REPOSITORY_MANIFEST_REL='.agents/results/task-13-final-tree-snapshot-20260902-090159.nul'
readonly IGNORED_INVENTORY_REL='.agents/results/task-13-final-ignored-input-inventory-20260902-090159.json'
readonly COVERAGE_ORACLE_REL='.agents/results/task-13-final-input-coverage-oracle-20260902-090159.json'
readonly SOURCE_HASH_REL='.agents/results/task-13-final-tree-snapshot-20260902-090159.sha256'

readonly -a GPE_IDS=(
  gpe-final-tree-repository-manifest
  gpe-ignored-input-inventory
  gpe-input-coverage-oracle
  gpe-source-snapshot-hash-artifact
)
readonly -a GPE_PATHS=(
  "$REPOSITORY_MANIFEST_REL"
  "$IGNORED_INVENTORY_REL"
  "$COVERAGE_ORACLE_REL"
  "$SOURCE_HASH_REL"
)
readonly -a GPE_FIELDS=(
  repository_manifest_sha256
  ignored_input_inventory_sha256
  input_coverage_oracle_sha256
  source_snapshot_hash_artifact_sha256
)

repo_root=''
manifest=''
assertion_digest=''

reject() { printf 'error: %s\n' "$1" >&2; exit 2; }
sha256_file() { sha256sum "$1" | awk '{print $1}'; }
sha256_stream() { sha256sum | awk '{print $1}'; }
identity() { stat -c '%d:%i' -- "$1"; }
mode() { stat -c '%a' -- "$1"; }
is_hex() { [[ "$1" =~ ^[0-9a-f]{64}$ ]]; }

usage() {
  printf '%s\n' \
    'usage: copied final-tree-record-r25.sh prepare-snapshot <digest>' \
    '       copied final-tree-record-r25.sh verify-snapshot <digest> <FV01|FV02|FV03|FV04|FV05|FV06|FV07|FV08>' \
    '       copied final-tree-record-r25.sh run-descriptor <digest> <x86_64-linux|aarch64-linux|aarch64-darwin> <gpe-map-digest> <first|fv07-replay>' >&2
  exit 2
}

physical_file() {
  local path="$1" directory physical_directory
  directory="$(dirname -- "$path")"
  physical_directory="$(cd -P -- "$directory" && pwd -P)" || return 1
  printf '%s/%s' "$physical_directory" "$(basename -- "$path")"
}

require_regular_nonlink() {
  local path="$1" label="$2"
  [[ -f "$path" && ! -L "$path" ]] || reject "$label is missing, not regular, or linked"
}

require_external_copy() {
  local path="$1" expected="$2" label="$3" physical
  [[ "$path" == /* ]] || reject "$label path is not absolute"
  require_regular_nonlink "$path" "$label"
  is_hex "$expected" || reject "$label hash receipt is malformed"
  physical="$(physical_file "$path")" || reject "$label path cannot be resolved"
  [[ "$physical" == "$path" ]] || reject "$label path is not physical"
  case "$physical" in "$repo_root"|"$repo_root"/*) reject "$label copy is inside the repository";; esac
  [[ "$(mode "$physical")" == 400 ]] || reject "$label copy mode is not 0400"
  [[ "$(sha256_file "$physical")" == "$expected" ]] || reject "$label copy hash differs from its receipt"
}

# jq ordinary parsing silently accepts duplicate members.  Run a streaming
# preparse first, then require exact compact sorted bytes plus one LF.
reject_duplicate_members() {
  local path="$1"
  jq --stream -n -e '
    reduce inputs as $item
      ({seen:{}, duplicate:false};
       if (($item|length) == 2 and ($item[0]|length) > 0) then
         ($item[0] | map(if type == "number" then "[\(.)]" else ".\(.)" end) | join("")) as $p
         | if .seen[$p] then .duplicate = true else .seen[$p] = true end
       else . end)
    | .duplicate == false
  ' "$path" >/dev/null 2>&1 || reject "duplicate or malformed JSON members in $path"
}

require_canonical_object() {
  local path="$1" label="$2"
  require_regular_nonlink "$path" "$label"
  reject_duplicate_members "$path"
  jq -e 'type == "object"' "$path" >/dev/null 2>&1 || reject "$label root is not an object"
  cmp -s <(jq -cS '.' "$path") "$path" || reject "$label bytes are not canonical jq -cS plus LF"
}

require_canonical_array() {
  local path="$1" label="$2"
  require_regular_nonlink "$path" "$label"
  reject_duplicate_members "$path"
  jq -e 'type == "array"' "$path" >/dev/null 2>&1 || reject "$label root is not an array"
  cmp -s <(jq -cS '.' "$path") "$path" || reject "$label bytes are not canonical jq -cS plus LF"
}

manifest_source_sha() {
  local relative="$1"
  jq -er --arg path "$relative" '
    [.command_source_integrity.sources[] | select(.path == $path)]
    | if length == 1
         and .[0].type == "regular_non_symlink"
         and (.[0].sha256 | test("^[0-9a-f]{64}$"))
      then .[0].sha256 else error("source row") end
  ' "$manifest"
}

verify_selected_justfile_receipts() {
  local selected="${TASK13_R25_SELECTED_JUSTFILE_COPY:-}"
  local selected_sha="${TASK13_R25_SELECTED_JUSTFILE_SHA256:-}"
  local selected_identity="${TASK13_R25_SELECTED_JUSTFILE_IDENTITY:-}"
  local expected_sha
  require_external_copy "$selected" "$selected_sha" 'selected R25 Justfile'
  expected_sha="$(manifest_source_sha "$SHAPE_REL")" || reject 'manifest selected-Justfile source row is invalid'
  [[ "$selected_sha" == "$expected_sha" ]] || reject 'selected R25 Justfile receipt is not manifest-bound'
  [[ "$selected_identity" =~ ^[0-9]+:[0-9]+$ && "$(identity "$selected")" == "$selected_identity" ]] || reject 'selected R25 Justfile identity receipt differs'
  [[ "${TASK13_BOUND_JUSTFILE:-}" == "$selected" \
    && "${TASK13_BOUND_JUSTFILE_SHA256:-}" == "$selected_sha" \
    && "${TASK13_BOUND_JUSTFILE_IDENTITY:-}" == "$selected_identity" ]] || reject 'generic selected-Justfile aliases differ from R25 receipts'
}

verify_bootstrap() {
  local digest="$1" self_copy self_sha strict_copy strict_sha invocation expected_self expected_strict
  is_hex "$digest" || reject 'assertion digest must be lowercase 64-hex'
  [[ "${TASK13_ASSERTION_GENERATION:-}" == "$GENERATION" ]] || reject 'guarded assertion generation receipt is missing or stale'
  [[ "${TASK13_ASSERTION_DIGEST:-}" == "$digest" ]] || reject 'guarded assertion digest receipt differs from argv'
  [[ -z "${TASK13_FLAKE_SOURCE_SNAPSHOT:-}" ]] || reject 'live R25 final-tree helper received an immutable-snapshot receipt'

  repo_root="$(pwd -P)"
  [[ "$repo_root" == /* && -d "$repo_root" ]] || reject 'physical repository root cannot be resolved'
  manifest="$repo_root/$MANIFEST_REL"
  assertion_digest="$digest"
  require_canonical_object "$manifest" 'R25 assertion manifest'
  [[ "$(sha256_file "$manifest")" == "$digest" ]] || reject 'live R25 manifest hash differs from argv'
  require_regular_nonlink "$repo_root/$LOCK_REL" 'R25 assertion lock'
  require_regular_nonlink "$repo_root/$RECORD_REL" 'R25 coordinator record'
  cmp -s <(printf '%s\n' "$digest") "$repo_root/$LOCK_REL" || reject 'R25 lock differs from assertion digest'
  cmp -s <(printf '%s\n' "$digest") "$repo_root/$RECORD_REL" || reject 'R25 coordinator record differs from assertion digest'

  jq -e --arg generation "$GENERATION" '
    .generation_id == $generation
    and .authority_binding.generation_id == $generation
    and .authority_binding.runtime_resolution == "copied_manifest_only"
    and .final_tree_snapshot.input_catalog_path == "tests/nix/task-13-final-input-catalog.json"
    and .final_tree_record_protocol.helper_abi.environment == [
      "TASK13_R25_FINAL_TREE_HELPER_COPY",
      "TASK13_R25_FINAL_TREE_HELPER_SHA256",
      "TASK13_R25_FINAL_VERIFY_HELPER_COPY",
      "TASK13_R25_FINAL_VERIFY_HELPER_SHA256",
      "TASK13_R25_STRICT_ENTRYPOINT_COPY",
      "TASK13_R25_STRICT_ENTRYPOINT_SHA256",
      "TASK13_R25_SELECTED_JUSTFILE_COPY",
      "TASK13_R25_SELECTED_JUSTFILE_SHA256",
      "TASK13_R25_SELECTED_JUSTFILE_IDENTITY",
      "TASK13_R25_GPE_MAP_SHA256"
    ]
    and .final_tree_record_protocol.helper_abi.generic_recipe_receipts == [
      "TASK13_ASSERTION_GENERATION",
      "TASK13_ASSERTION_DIGEST",
      "TASK13_FLAKE_SOURCE_SNAPSHOT",
      "TASK13_BOUND_JUSTFILE",
      "TASK13_BOUND_JUSTFILE_IDENTITY",
      "TASK13_BOUND_JUSTFILE_SHA256"
    ]
    and .final_tree_record_protocol.terminal_schema.record_fields == [
      "session","assertion_digest","source_snapshot","gpe_artifact_hash_map",
      "gpe_artifact_hash_map_sha256","target_descriptor_id","descriptor_digest",
      "capability_evidence","reservation_id","terminal_state","terminal_failure_class",
      "original_invocation_count","invocation_id","command_identity","start_timestamp",
      "end_timestamp","raw_exit","raw_evidence_reference","post_action_source_snapshot",
      "post_action_ignored_inventory_sha256","post_action_generated_evidence_binding_sha256",
      "post_action_gpe_artifact_hash_map","post_action_gpe_artifact_hash_map_sha256",
      "post_action_descriptor_digest","attribute_to_output_path","record_sha256"
    ]
  ' "$manifest" >/dev/null || reject 'materialized R25 helper protocol is malformed'

  self_copy="${TASK13_R25_FINAL_TREE_HELPER_COPY:-}"
  self_sha="${TASK13_R25_FINAL_TREE_HELPER_SHA256:-}"
  invocation="$(physical_file "${BASH_SOURCE[0]}")" || reject 'helper invocation path cannot be resolved'
  [[ "$invocation" == "$self_copy" ]] || reject 'final-tree helper was not invoked from the guarded external copy'
  require_external_copy "$self_copy" "$self_sha" 'final-tree helper'
  expected_self="$(manifest_source_sha "$SELF_REL")" || reject 'manifest final-tree source row is invalid'
  [[ "$self_sha" == "$expected_self" ]] || reject 'final-tree helper receipt is not manifest-bound'

  strict_copy="${TASK13_R25_STRICT_ENTRYPOINT_COPY:-}"
  strict_sha="${TASK13_R25_STRICT_ENTRYPOINT_SHA256:-}"
  require_external_copy "$strict_copy" "$strict_sha" 'strict entrypoint'
  expected_strict="$(manifest_source_sha "$STRICT_REL")" || reject 'manifest strict-entrypoint source row is invalid'
  [[ "$strict_sha" == "$expected_strict" ]] || reject 'strict-entrypoint receipt is not manifest-bound'
  verify_selected_justfile_receipts
}

validate_catalog() {
  local catalog="$repo_root/$CATALOG_REL"
  require_canonical_array "$catalog" 'final input catalog'
  jq -e --argjson required "$(jq -c '.final_tree_snapshot.catalog_required_entries' "$manifest")" '
    all(.[];
      (
        ((has("path_or_external_selector") and (has("literal_external_selector")|not))
          and (keys | sort) == (["classification","consuming_stages","input_id","path_or_external_selector","producer","sha256_rule","subkind"] | sort)
          and (.path_or_external_selector | type == "string" and length > 0))
        or
        ((has("literal_external_selector") and (has("path_or_external_selector")|not))
          and (keys | sort) == (["authoring_trace_ref","classification","consuming_stages","input_id","literal_external_selector","producer","sha256_rule","subkind"] | sort)
          and (.literal_external_selector | type == "object")
          and (.authoring_trace_ref | type == "string" and length > 0))
      )
      and (.input_id | type == "string" and length > 0)
      and (.classification == "repository_snapshot" or .classification == "explicit_ignored_evidence" or .classification == "immutable_protocol_output")
      and (.subkind | type == "string" and length > 0)
      and (.producer | type == "string" and length > 0)
      and (.sha256_rule | type == "string" and length > 0)
      and (.consuming_stages | type == "array" and length > 0 and all(.[]; type == "string" and length > 0)))
    and ([.[].input_id] | length == (unique | length))
    and ([.[].input_id] == $required)
    and ((([.[].subkind] | unique) - ["audit_result","external_baseline","generated_protocol_evidence","handoff","preexisting_evidence","raw_result","receipt","repository_path","reservation","terminal_record"]) == [])
  ' "$catalog" >/dev/null || reject 'final input catalog schema or required-entry order differs from R25 manifest'
  jq -e --slurpfile catalog_file "$catalog" '
    ($catalog_file[0]) as $catalog
    |
    .final_tree_record_protocol.fv_input_sets as $inputs
    | .final_tree_record_protocol.fv_output_sets as $outputs
    | all($inputs.canonical_order[] as $fv;
        (($inputs[$fv].immutable_protocol_output_input_ids - $outputs[$fv])
         | length) == ($inputs[$fv].immutable_protocol_output_input_ids | length))
    and all($inputs.canonical_order[] as $fv;
        all($inputs[$fv].immutable_protocol_output_input_ids[];
          . as $id | any($catalog[]; .input_id == $id and .classification == "immutable_protocol_output")))
  ' "$manifest" >/dev/null || reject 'FV input catalog has a same-step cycle or unresolved immutable input'
  jq -e --slurpfile catalog_file "$catalog" '
    ($catalog_file[0]) as $catalog
    | (.final_tree_snapshot.catalog_fixed_entries | length) as $fixed_count
    | (.final_tree_snapshot.generated_protocol_evidence.artifacts | length) as $gpe_count
    | ($catalog[0:$fixed_count] == .final_tree_snapshot.catalog_fixed_entries)
      and ($catalog[($fixed_count+$gpe_count):] == .final_tree_snapshot.catalog_protocol_output_entries)
      and all(range(0;$gpe_count) as $index;
        ($catalog[$fixed_count+$index].input_id == .final_tree_snapshot.generated_protocol_evidence.artifacts[$index].input_id)
        and ($catalog[$fixed_count+$index].path_or_external_selector == .final_tree_snapshot.generated_protocol_evidence.artifacts[$index].path)
        and ($catalog[$fixed_count+$index].classification == "immutable_protocol_output")
        and ($catalog[$fixed_count+$index].subkind == "generated_protocol_evidence"))
  ' "$manifest" >/dev/null || reject 'final input catalog differs from the materialized fixed/GPE/protocol-output authority'
  jq -e '
    .final_tree_snapshot.catalog_dispatch_entries as $catalog_dispatch
    | .operator_surfaces.final_verify.dispatch_by_id as $dispatch_by_id
    | [.operator_surfaces.final_verify.inventory[]
        | select(.id != "fv-09-redacted-no-mutation-closeout")] as $inventory
    | ($catalog_dispatch | length) == 8
    and ($inventory | length) == 8
    and (($dispatch_by_id | keys | map(select(. != "fv-09-redacted-no-mutation-closeout")))
         == [$catalog_dispatch[].dispatch_id])
    and ([$inventory[].id] == [$catalog_dispatch[].dispatch_id])
    and all($catalog_dispatch[];
      (keys | sort) == (["classification","consuming_stages","dispatch_id",
        "exact_ordered_command_strings","input_id","internal_ordered_command_strings",
        "json_pointer","owning_tracked_path","producer","sha256_rule","source_bindings",
        "subkind"] | sort)
      and .classification == "repository_snapshot"
      and .subkind == "dispatch_descriptor"
      and (.exact_ordered_command_strings | type == "array" and length > 0
           and all(.[]; type == "string" and length > 0))
      and (.internal_ordered_command_strings | type == "array" and length > 0
           and all(.[]; type == "string" and length > 0))
      and (.source_bindings | type == "array" and length > 0
           and all(.[]; type == "string" and length > 0)))
    and all(range(0;8);
      . as $index
      | ($catalog_dispatch[$index].dispatch_id) as $id
      | $catalog_dispatch[$index].exact_ordered_command_strings == $dispatch_by_id[$id].dispatch
      and $catalog_dispatch[$index].internal_ordered_command_strings == $dispatch_by_id[$id].internal_dispatch
      and $catalog_dispatch[$index].exact_ordered_command_strings == $inventory[$index].dispatch
      and $catalog_dispatch[$index].internal_ordered_command_strings == $inventory[$index].internal_dispatch)
  ' "$manifest" >/dev/null || reject 'FV dispatch catalogs, operator map, and ordered inventory differ'
}

is_protocol_output_path() {
  local candidate="$1"
  case "$candidate" in
    "$REPOSITORY_MANIFEST_REL"|"$IGNORED_INVENTORY_REL"|"$COVERAGE_ORACLE_REL"|"$SOURCE_HASH_REL"|\
    .agents/results/.task-13-r25-*-reservation-20260907|\
    .agents/results/.task-13-r25-*-reservation-20260907/*|\
    .agents/results/.task-13-r25-fv*-stage.*|\
    .agents/results/.task-13-r25-fv*-stage.*/*|\
    .agents/results/task-13-r25-*-terminal-20260907.json|\
    .agents/results/task-13-r25-*-first-receipt-20260907.json|\
    .agents/results/task-13-r25-*-fv07-replay-receipt-20260907.json|\
    .agents/results/task-13-r25-final-verify-handoff-20260907.json|\
    .agents/results/task-13-fv01-pf1-raw-result-20260902-090159.txt|\
    .agents/results/task-13-fv01-observed-legacy-snapshot-20260902-090159.nul|\
    .agents/results/task-13-fv02-matrix-raw-result-20260902-090159.txt|\
    .agents/results/task-13-fv03-consolidated-whole-tree-20260902-090159.txt|\
    .agents/results/task-13-fv04-recovery-raw-result-20260902-090159.txt|\
    .agents/results/task-13-fv05-migration-adr-audit-20260902-090159.json|\
    .agents/results/task-13-fv06-c2-provenance-20260902-090159.json|\
    .agents/results/task-13-fv07-builder-matrix-raw-result-20260902-090159.txt|\
    .agents/results/task-13-fv08-dependency-20260902-090159.txt|\
    .agents/results/task-13-fv08-license-20260902-090159.txt|\
    .agents/results/task-13-fv08-sensitive-log-gitleaks-20260902-090159.txt)
      return 0 ;;
  esac
  return 1
}

repository_entry() {
  local relative="$1" absolute="$repo_root/$relative" kind file_mode content_sha stage
  [[ "$relative" != /* && "$relative" != *$'\0'* ]] || reject 'repository snapshot contains an invalid path'
  if [[ -L "$absolute" ]]; then
    kind='symlink'
    file_mode="$(mode "$absolute")"
    content_sha="$(printf '%s' "$(readlink -- "$absolute")" | sha256_stream)"
  elif [[ -f "$absolute" ]]; then
    kind='regular'
    file_mode="$(mode "$absolute")"
    content_sha="$(sha256_file "$absolute")"
  elif [[ ! -e "$absolute" ]]; then
    kind='missing'
    stage="$(git -C "$repo_root" ls-files --stage -- "$relative" | LC_ALL=C head -n 1)"
    file_mode="${stage%% *}"
    [[ "$file_mode" =~ ^[0-9]{6}$ ]] || file_mode='missing'
    content_sha="$(printf '%s' 'missing' | sha256_stream)"
  elif [[ -d "$absolute" ]]; then
    kind='gitlink'
    stage="$(git -C "$repo_root" ls-files --stage -- "$relative" | LC_ALL=C head -n 1)"
    file_mode="${stage%% *}"
    [[ "$file_mode" =~ ^[0-9]{6}$ ]] || reject "untracked directory entered repository snapshot: $relative"
    content_sha="$(printf '%s' "$stage" | sha256_stream)"
  else
    reject "unsupported repository input kind: $relative"
  fi
  printf '%s\0%s\0%s\0%s\0' "$relative" "$kind" "$file_mode" "$content_sha"
}

build_repository_manifest() {
  local destination="$1" list_file
  list_file="$(mktemp "${TMPDIR:-/tmp}/jamye-task13-r25-repo-list.XXXXXX")" || reject 'cannot stage repository path list'
  git -C "$repo_root" ls-files --cached --others --exclude-standard -z \
    | LC_ALL=C sort -zu >"$list_file" || reject 'cannot enumerate repository snapshot inputs'
  : >"$destination"
  while IFS= read -r -d '' relative; do
    is_protocol_output_path "$relative" && continue
    repository_entry "$relative" >>"$destination"
  done <"$list_file"
  [[ -s "$destination" ]] || reject 'repository snapshot manifest is empty'
  rm -f -- "$list_file"
}

build_ignored_inventory() {
  local destination="$1" catalog="$repo_root/$CATALOG_REL" rows input_id selector selector_json absolute byte_sha role producer
  rows="$(mktemp "${TMPDIR:-/tmp}/jamye-task13-r25-ignored.XXXXXX")" || reject 'cannot stage ignored inventory rows'
  : >"$rows"
  while IFS= read -r input_id; do
    role="$(jq -er --arg id "$input_id" '.[] | select(.input_id == $id) | .subkind' "$catalog")"
    producer="$(jq -er --arg id "$input_id" '.[] | select(.input_id == $id) | .producer' "$catalog")"
    if jq -e --arg id "$input_id" '.[] | select(.input_id == $id) | has("path_or_external_selector")' "$catalog" >/dev/null; then
      selector="$(jq -er --arg id "$input_id" '.[] | select(.input_id == $id) | .path_or_external_selector' "$catalog")"
      if [[ "$selector" == /* ]]; then absolute="$selector"; else absolute="$repo_root/$selector"; fi
      require_regular_nonlink "$absolute" "ignored evidence $input_id"
      byte_sha="$(sha256_file "$absolute")"
    else
      selector_json="$(jq -cS --arg id "$input_id" '.[] | select(.input_id == $id) | .literal_external_selector' "$catalog")"
      selector="literal:$input_id"
      byte_sha="$(printf '%s\n' "$selector_json" | sha256_stream)"
    fi
    jq -cnS --arg classification explicit_ignored_evidence --arg input_id "$input_id" \
      --arg path "$selector" --arg producer "$producer" --arg role "$role" --arg sha256 "$byte_sha" \
      '{classification:$classification,input_id:$input_id,path:$path,producer:$producer,role:$role,sha256:$sha256}' >>"$rows"
  done < <(jq -r '.[] | select(.classification == "explicit_ignored_evidence") | .input_id' "$catalog")
  jq -cS -s '{entries:.,schema_version:"1.0"}' "$rows" >"$destination"
  rm -f -- "$rows"
}

build_coverage_oracle() {
  local destination="$1" catalog="$repo_root/$CATALOG_REL" repository_sha="$2" ignored_sha="$3"
  jq -cS --arg repository_sha "$repository_sha" --arg ignored_sha "$ignored_sha" '
    . as $catalog
    | {
        classifications: ($catalog | group_by(.classification) | map({classification:.[0].classification,count:length})),
        consumed_input_ids: ($catalog | map(.input_id)),
        ignored_input_inventory_sha256: $ignored_sha,
        repository_manifest_sha256: $repository_sha,
        schema_version: "1.0",
        subkinds: ($catalog | map(.subkind) | unique),
        verdict: "PASS"
      }
  ' "$catalog" >"$destination"
}

binding_payload_sha() {
  local repository_sha="$1" ignored_sha="$2" oracle_sha="$3" descriptor_ids
  descriptor_ids="$(jq -cS '[.final_tree_snapshot.generated_protocol_evidence.artifacts[].input_id]' "$manifest")"
  printf '%s\0%s\0%s\0%s\0%s\0%s\0%s\0%s\0%s\0%s\0' \
    binding_version 1 \
    tracked_generated_descriptor_ids "$descriptor_ids" \
    repository_manifest_sha256 "$repository_sha" \
    ignored_input_inventory_sha256 "$ignored_sha" \
    input_coverage_oracle_sha256 "$oracle_sha" | sha256_stream
}

write_source_hash_artifact() {
  local destination="$1" repository_sha="$2" ignored_sha="$3" oracle_sha="$4" binding_sha="$5" descriptor_ids
  descriptor_ids="$(jq -cS '[.final_tree_snapshot.generated_protocol_evidence.artifacts[].input_id]' "$manifest")"
  printf '%s\0%s\0%s\0%s\0%s\0%s\0%s\0%s\0%s\0%s\0%s\0%s\0' \
    binding_version 1 \
    tracked_generated_descriptor_ids "$descriptor_ids" \
    repository_manifest_sha256 "$repository_sha" \
    ignored_input_inventory_sha256 "$ignored_sha" \
    input_coverage_oracle_sha256 "$oracle_sha" \
    generated_evidence_binding_sha256 "$binding_sha" >"$destination"
}

source_snapshot_sha() {
  local repository_sha="$1" ignored_sha="$2" oracle_sha="$3" binding_sha="$4"
  printf '%s\0%s\0%s\0%s\0%s\0%s\0%s\0%s\0' \
    repository_manifest_sha256 "$repository_sha" \
    ignored_input_inventory_sha256 "$ignored_sha" \
    input_coverage_oracle_sha256 "$oracle_sha" \
    generated_evidence_binding_sha256 "$binding_sha" | sha256_stream
}

gpe_map_sha() {
  local map_json="$1"
  while IFS=$'\t' read -r input_id path field byte_sha; do
    printf '%s\0%s\0%s\0%s\0' "$input_id" "$path" "$field" "$byte_sha"
  done < <(jq -r '.[] | [.input_id,.path,.sha256_field,.byte_sha256] | @tsv' <<<"$map_json") | sha256_stream
}

make_gpe_map() {
  local repository_sha="$1" ignored_sha="$2" oracle_sha="$3" source_hash_sha="$4"
  jq -cnS \
    --arg r "$repository_sha" --arg i "$ignored_sha" --arg o "$oracle_sha" --arg s "$source_hash_sha" '
    [
      {byte_sha256:$r,input_id:"gpe-final-tree-repository-manifest",path:".agents/results/task-13-final-tree-snapshot-20260902-090159.nul",sha256_field:"repository_manifest_sha256"},
      {byte_sha256:$i,input_id:"gpe-ignored-input-inventory",path:".agents/results/task-13-final-ignored-input-inventory-20260902-090159.json",sha256_field:"ignored_input_inventory_sha256"},
      {byte_sha256:$o,input_id:"gpe-input-coverage-oracle",path:".agents/results/task-13-final-input-coverage-oracle-20260902-090159.json",sha256_field:"input_coverage_oracle_sha256"},
      {byte_sha256:$s,input_id:"gpe-source-snapshot-hash-artifact",path:".agents/results/task-13-final-tree-snapshot-20260902-090159.sha256",sha256_field:"source_snapshot_hash_artifact_sha256"}
    ]'
}

exclusive_publish() {
  local staged="$1" destination="$2" label="$3"
  [[ ! -e "$destination" && ! -L "$destination" ]] || reject "$label already exists"
  ln -- "$staged" "$destination" || reject "$label exclusive publication failed"
  cmp -s "$staged" "$destination" || reject "$label differs immediately after publication"
}

generate_mutable_snapshot() {
  local directory="$1" repository_file ignored_file oracle_file source_file
  local repository_sha ignored_sha oracle_sha binding_sha source_sha source_file_sha map_json map_sha
  repository_file="$directory/repository.nul"
  ignored_file="$directory/ignored.json"
  oracle_file="$directory/oracle.json"
  source_file="$directory/source.sha256"
  build_repository_manifest "$repository_file"
  repository_sha="$(sha256_file "$repository_file")"
  build_ignored_inventory "$ignored_file"
  ignored_sha="$(sha256_file "$ignored_file")"
  build_coverage_oracle "$oracle_file" "$repository_sha" "$ignored_sha"
  oracle_sha="$(sha256_file "$oracle_file")"
  binding_sha="$(binding_payload_sha "$repository_sha" "$ignored_sha" "$oracle_sha")"
  write_source_hash_artifact "$source_file" "$repository_sha" "$ignored_sha" "$oracle_sha" "$binding_sha"
  source_file_sha="$(sha256_file "$source_file")"
  source_sha="$(source_snapshot_sha "$repository_sha" "$ignored_sha" "$oracle_sha" "$binding_sha")"
  map_json="$(make_gpe_map "$repository_sha" "$ignored_sha" "$oracle_sha" "$source_file_sha")"
  map_sha="$(gpe_map_sha "$map_json")"
  jq -cnS --argjson gpe_artifact_hash_map "$map_json" \
    --arg gpe_artifact_hash_map_sha256 "$map_sha" \
    --arg repository_manifest_sha256 "$repository_sha" \
    --arg ignored_input_inventory_sha256 "$ignored_sha" \
    --arg input_coverage_oracle_sha256 "$oracle_sha" \
    --arg source_snapshot_hash_artifact_sha256 "$source_file_sha" \
    --arg generated_evidence_binding_sha256 "$binding_sha" \
    --arg source_snapshot "$source_sha" \
    '{generated_evidence_binding_sha256:$generated_evidence_binding_sha256,gpe_artifact_hash_map:$gpe_artifact_hash_map,gpe_artifact_hash_map_sha256:$gpe_artifact_hash_map_sha256,ignored_input_inventory_sha256:$ignored_input_inventory_sha256,input_coverage_oracle_sha256:$input_coverage_oracle_sha256,repository_manifest_sha256:$repository_manifest_sha256,source_snapshot:$source_snapshot,source_snapshot_hash_artifact_sha256:$source_snapshot_hash_artifact_sha256}'
}

verify_published_gpe_shape() {
  local index path expected_id expected_field
  for index in 0 1 2 3; do
    path="$repo_root/${GPE_PATHS[$index]}"
    require_regular_nonlink "$path" "published GPE ${GPE_IDS[$index]}"
    expected_id="$(jq -er ".final_tree_snapshot.generated_protocol_evidence.artifacts[$index].input_id" "$manifest")"
    expected_field="$(jq -er ".final_tree_snapshot.generated_protocol_evidence.artifacts[$index].sha256_field" "$manifest")"
    [[ "$expected_id" == "${GPE_IDS[$index]}" && "$expected_field" == "${GPE_FIELDS[$index]}" ]] || reject 'manifest GPE order or field set differs from frozen protocol'
    [[ "$(jq -er ".final_tree_snapshot.generated_protocol_evidence.artifacts[$index].path" "$manifest")" == "${GPE_PATHS[$index]}" ]] || reject 'manifest GPE path differs from frozen protocol'
  done
  require_canonical_object "$repo_root/$IGNORED_INVENTORY_REL" 'published ignored inventory'
  require_canonical_object "$repo_root/$COVERAGE_ORACLE_REL" 'published coverage oracle'
}

snapshot_state() {
  local temp current published_repository_sha published_ignored_sha published_oracle_sha published_source_file_sha
  local current_repository_sha current_ignored_sha current_oracle_sha current_binding_sha current_source_sha
  local stored_binding_sha stored_source_sha map_json map_sha current_equal
  verify_published_gpe_shape
  published_repository_sha="$(sha256_file "$repo_root/$REPOSITORY_MANIFEST_REL")"
  published_ignored_sha="$(sha256_file "$repo_root/$IGNORED_INVENTORY_REL")"
  published_oracle_sha="$(sha256_file "$repo_root/$COVERAGE_ORACLE_REL")"
  published_source_file_sha="$(sha256_file "$repo_root/$SOURCE_HASH_REL")"
  map_json="$(make_gpe_map "$published_repository_sha" "$published_ignored_sha" "$published_oracle_sha" "$published_source_file_sha")"
  map_sha="$(gpe_map_sha "$map_json")"

  stored_binding_sha="$(tr '\0' '\n' <"$repo_root/$SOURCE_HASH_REL" | awk 'p=="generated_evidence_binding_sha256"{print;exit}{p=$0}')"
  is_hex "$stored_binding_sha" || reject 'published source-hash artifact has no valid displayed binding digest'
  [[ "$(binding_payload_sha "$published_repository_sha" "$published_ignored_sha" "$published_oracle_sha")" == "$stored_binding_sha" ]] || reject 'published generated-evidence binding digest is invalid'
  stored_source_sha="$(source_snapshot_sha "$published_repository_sha" "$published_ignored_sha" "$published_oracle_sha" "$stored_binding_sha")"

  temp="$(mktemp -d "${TMPDIR:-/tmp}/jamye-task13-r25-state.XXXXXX")" || reject 'cannot stage current snapshot verification'
  current="$(generate_mutable_snapshot "$temp")"
  current_repository_sha="$(jq -er '.repository_manifest_sha256' <<<"$current")"
  current_ignored_sha="$(jq -er '.ignored_input_inventory_sha256' <<<"$current")"
  current_oracle_sha="$(jq -er '.input_coverage_oracle_sha256' <<<"$current")"
  current_binding_sha="$(jq -er '.generated_evidence_binding_sha256' <<<"$current")"
  current_source_sha="$(jq -er '.source_snapshot' <<<"$current")"
  if cmp -s "$temp/repository.nul" "$repo_root/$REPOSITORY_MANIFEST_REL" \
    && cmp -s "$temp/ignored.json" "$repo_root/$IGNORED_INVENTORY_REL" \
    && cmp -s "$temp/oracle.json" "$repo_root/$COVERAGE_ORACLE_REL" \
    && cmp -s "$temp/source.sha256" "$repo_root/$SOURCE_HASH_REL" \
    && [[ "$current_binding_sha" == "$stored_binding_sha" && "$current_source_sha" == "$stored_source_sha" ]]; then
    current_equal=true
  else
    current_equal=false
  fi
  rm -rf -- "$temp"
  jq -cnS --argjson current_equal "$current_equal" --argjson gpe_artifact_hash_map "$map_json" \
    --arg gpe_artifact_hash_map_sha256 "$map_sha" \
    --arg repository_manifest_sha256 "$published_repository_sha" \
    --arg ignored_input_inventory_sha256 "$published_ignored_sha" \
    --arg input_coverage_oracle_sha256 "$published_oracle_sha" \
    --arg source_snapshot_hash_artifact_sha256 "$published_source_file_sha" \
    --arg generated_evidence_binding_sha256 "$stored_binding_sha" \
    --arg source_snapshot "$stored_source_sha" \
    --arg current_repository_manifest_sha256 "$current_repository_sha" \
    --arg current_ignored_input_inventory_sha256 "$current_ignored_sha" \
    --arg current_input_coverage_oracle_sha256 "$current_oracle_sha" \
    --arg current_generated_evidence_binding_sha256 "$current_binding_sha" \
    --arg current_source_snapshot "$current_source_sha" \
    '{current_equal:$current_equal,current_generated_evidence_binding_sha256:$current_generated_evidence_binding_sha256,current_ignored_input_inventory_sha256:$current_ignored_input_inventory_sha256,current_input_coverage_oracle_sha256:$current_input_coverage_oracle_sha256,current_repository_manifest_sha256:$current_repository_manifest_sha256,current_source_snapshot:$current_source_snapshot,generated_evidence_binding_sha256:$generated_evidence_binding_sha256,gpe_artifact_hash_map:$gpe_artifact_hash_map,gpe_artifact_hash_map_sha256:$gpe_artifact_hash_map_sha256,ignored_input_inventory_sha256:$ignored_input_inventory_sha256,input_coverage_oracle_sha256:$input_coverage_oracle_sha256,repository_manifest_sha256:$repository_manifest_sha256,source_snapshot:$source_snapshot,source_snapshot_hash_artifact_sha256:$source_snapshot_hash_artifact_sha256}'
}

require_current_snapshot() {
  local expected_map_sha="$1" state
  state="$(snapshot_state)"
  jq -e --arg expected "$expected_map_sha" '.current_equal == true and .gpe_artifact_hash_map_sha256 == $expected' <<<"$state" >/dev/null \
    || reject 'current source/GPE state differs from the immutable snapshot or requested map digest'
  printf '%s\n' "$state"
}

prepare_snapshot() {
  local reservation_rel reservation reservation_id stage before after index
  local repository_sha ignored_sha oracle_sha source_file_sha map_json map_sha binding_sha source_sha
  validate_catalog
  reservation_rel="$(jq -er '.final_tree_record_protocol.reservation_abi.snapshot_path' "$manifest")"
  [[ "$reservation_rel" == '.agents/results/.task-13-r25-snapshot-reservation-20260907' ]] || reject 'snapshot reservation path differs from R25 ABI'
  reservation="$repo_root/$reservation_rel"
  for index in 0 1 2 3; do
    [[ ! -e "$repo_root/${GPE_PATHS[$index]}" && ! -L "$repo_root/${GPE_PATHS[$index]}" ]] || reject 'a generated proof-evidence artifact already exists'
  done
  reservation_id="$(printf '%s\0%s\0%s\0' "$GENERATION" "$assertion_digest" prepare-snapshot | sha256_stream)"
  mkdir -- "$reservation" || reject 'snapshot reservation already exists or cannot be acquired'
  stage="$reservation/staged"
  mkdir -- "$stage" || reject 'snapshot reservation staging directory cannot be created'
  printf '%s\n' "$reservation_id" >"$reservation/reservation-id"
  chmod 0400 -- "$reservation/reservation-id"

  before="$(generate_mutable_snapshot "$stage")"
  repository_sha="$(jq -er '.repository_manifest_sha256' <<<"$before")"
  ignored_sha="$(jq -er '.ignored_input_inventory_sha256' <<<"$before")"
  oracle_sha="$(jq -er '.input_coverage_oracle_sha256' <<<"$before")"
  source_file_sha="$(jq -er '.source_snapshot_hash_artifact_sha256' <<<"$before")"
  map_json="$(jq -cS '.gpe_artifact_hash_map' <<<"$before")"
  map_sha="$(jq -er '.gpe_artifact_hash_map_sha256' <<<"$before")"
  binding_sha="$(jq -er '.generated_evidence_binding_sha256' <<<"$before")"
  source_sha="$(jq -er '.source_snapshot' <<<"$before")"

  # Mutable source/ignored inputs must still be byte-identical immediately
  # before publication.
  mkdir -- "$reservation/prepublish"
  after="$(generate_mutable_snapshot "$reservation/prepublish")"
  [[ "$after" == "$before" ]] || reject 'repository or ignored input drifted before GPE publication'
  chmod 0400 -- "$stage/repository.nul" "$stage/ignored.json" "$stage/oracle.json" "$stage/source.sha256"
  exclusive_publish "$stage/repository.nul" "$repo_root/$REPOSITORY_MANIFEST_REL" 'repository-manifest GPE'
  exclusive_publish "$stage/ignored.json" "$repo_root/$IGNORED_INVENTORY_REL" 'ignored-inventory GPE'
  exclusive_publish "$stage/oracle.json" "$repo_root/$COVERAGE_ORACLE_REL" 'coverage-oracle GPE'
  exclusive_publish "$stage/source.sha256" "$repo_root/$SOURCE_HASH_REL" 'source-hash GPE'

  # Reopen every published byte, validate the exact map, and revalidate all
  # mutable inputs.  A partial publication is intentionally never reclaimed.
  [[ "$(sha256_file "$repo_root/$REPOSITORY_MANIFEST_REL")" == "$repository_sha" \
    && "$(sha256_file "$repo_root/$IGNORED_INVENTORY_REL")" == "$ignored_sha" \
    && "$(sha256_file "$repo_root/$COVERAGE_ORACLE_REL")" == "$oracle_sha" \
    && "$(sha256_file "$repo_root/$SOURCE_HASH_REL")" == "$source_file_sha" ]] || reject 'published GPE bytes differ from staged bytes'
  after="$(require_current_snapshot "$map_sha")"
  [[ "$(jq -er '.source_snapshot' <<<"$after")" == "$source_sha" \
    && "$(jq -er '.generated_evidence_binding_sha256' <<<"$after")" == "$binding_sha" ]] || reject 'postpublication source/binding differs from staged snapshot'

  jq -cnS --argjson gpe_artifact_hash_map "$map_json" \
    --arg gpe_artifact_hash_map_sha256 "$map_sha" \
    --arg repository_manifest_sha256 "$repository_sha" \
    --arg ignored_input_inventory_sha256 "$ignored_sha" \
    --arg input_coverage_oracle_sha256 "$oracle_sha" \
    --arg source_snapshot_hash_artifact_sha256 "$source_file_sha" \
    --arg generated_evidence_binding_sha256 "$binding_sha" \
    --arg source_snapshot "$source_sha" \
    '{generated_evidence_binding_sha256:$generated_evidence_binding_sha256,gpe_artifact_hash_map:$gpe_artifact_hash_map,gpe_artifact_hash_map_sha256:$gpe_artifact_hash_map_sha256,ignored_input_inventory_sha256:$ignored_input_inventory_sha256,input_coverage_oracle_sha256:$input_coverage_oracle_sha256,repository_manifest_sha256:$repository_manifest_sha256,source_snapshot:$source_snapshot,source_snapshot_hash_artifact_sha256:$source_snapshot_hash_artifact_sha256}'
}

catalog_output_hash() {
  local input_id="$1" path classification subkind absolute displayed
  path="$(jq -er --arg id "$input_id" '.[] | select(.input_id == $id) | .path_or_external_selector' "$repo_root/$CATALOG_REL")" || reject "catalog output path is missing for $input_id"
  classification="$(jq -er --arg id "$input_id" '.[] | select(.input_id == $id) | .classification' "$repo_root/$CATALOG_REL")"
  subkind="$(jq -er --arg id "$input_id" '.[] | select(.input_id == $id) | .subkind' "$repo_root/$CATALOG_REL")"
  [[ "$classification" == immutable_protocol_output && -n "$path" ]] || reject "catalog input is not a concrete immutable output: $input_id"
  if [[ "$path" == /* ]]; then absolute="$path"; else absolute="$repo_root/$path"; fi
  require_regular_nonlink "$absolute" "immutable protocol input $input_id"
  case "$subkind" in
    terminal_record)
      require_canonical_object "$absolute" "$input_id"
      displayed="$(jq -er '.record_sha256 | select(test("^[0-9a-f]{64}$"))' "$absolute")"
      [[ "$(json_payload_hash "$absolute" record_sha256)" == "$displayed" ]] || reject "$input_id terminal payload hash differs"
      printf '%s\n' "$displayed" ;;
    receipt)
      require_canonical_object "$absolute" "$input_id"
      displayed="$(jq -er '.receipt_sha256 | select(test("^[0-9a-f]{64}$"))' "$absolute")"
      [[ "$(json_payload_hash "$absolute" receipt_sha256)" == "$displayed" ]] || reject "$input_id receipt payload hash differs"
      printf '%s\n' "$displayed" ;;
    *) sha256_file "$absolute" ;;
  esac
}

resolved_dispatch_descriptor() {
  local fv="$1" index dispatch expected_id dispatch_id owning owning_sha path sha source_rows='[]' descriptor_sha
  index="$(jq -r --arg fv "$fv" '.final_tree_record_protocol.fv_input_sets.canonical_order | index($fv)' "$manifest")"
  [[ "$index" =~ ^[0-7]$ ]] || reject "FV dispatch index is invalid for $fv"
  dispatch="$(jq -ce --argjson index "$index" '.final_tree_snapshot.catalog_dispatch_entries[$index]' "$manifest")"
  expected_id="$(jq -r --arg fv "$fv" '.final_tree_record_protocol.fv_input_sets[$fv].descriptor_dispatch_input_ids[0]' "$manifest")"
  dispatch_id="$(jq -r '.input_id' <<<"$dispatch")"
  [[ "$dispatch_id" == "$expected_id" ]] || reject "FV dispatch descriptor ID differs for $fv"
  owning="$(jq -r '.owning_tracked_path' <<<"$dispatch")"
  owning_sha="$(manifest_source_sha "$owning")" || reject "FV owning source is not manifest-bound for $fv"
  while IFS= read -r path; do
    sha="$(manifest_source_sha "$path")" || reject "FV dispatch source binding is unresolved for $fv"
    source_rows="$(jq -cnS --argjson rows "$source_rows" --arg path "$path" --arg sha256 "$sha" '$rows + [{path:$path,sha256:$sha256}]')"
  done < <(jq -r '.source_bindings[]' <<<"$dispatch")
  descriptor_sha="$({
    while IFS= read -r sha; do printf '%s\0' "$sha"; done < <(jq -r '.[].sha256' <<<"$source_rows")
    printf '%s\0%s\0%s\0' "$(jq -r '.dispatch_id' <<<"$dispatch")" "$(jq -cS '.exact_ordered_command_strings' <<<"$dispatch")" "$(jq -cS '.internal_ordered_command_strings' <<<"$dispatch")"
  } | sha256_stream)"
  jq -cnS --arg dispatch_id "$(jq -r '.dispatch_id' <<<"$dispatch")" --arg input_id "$dispatch_id" \
    --arg owning_tracked_path "$owning" --arg owning_tracked_path_sha256 "$owning_sha" \
    --arg descriptor_sha256 "$descriptor_sha" --argjson source_bindings "$source_rows" \
    --argjson exact "$(jq -c '.exact_ordered_command_strings' <<<"$dispatch")" \
    --argjson internal "$(jq -c '.internal_ordered_command_strings' <<<"$dispatch")" \
    '{descriptor_sha256:$descriptor_sha256,dispatch_id:$dispatch_id,exact_ordered_command_strings:$exact,input_id:$input_id,internal_ordered_command_strings:$internal,owning_tracked_path:$owning_tracked_path,owning_tracked_path_sha256:$owning_tracked_path_sha256,source_bindings:$source_bindings}'
}

repository_ids_json() {
  jq -cRs '
    split("\u0000") as $fields
    | if (($fields|length) >= 5 and (($fields|length)-1) % 4 == 0 and $fields[-1] == "")
      then [range(0;($fields|length)-1;4) | "repo:" + $fields[.]]
      else error("malformed repository manifest tuple stream") end
  ' "$repo_root/$REPOSITORY_MANIFEST_REL"
}

repository_tuple_stream() {
  local path kind file_mode content_sha entry_sha previous=''
  while IFS= read -r -d '' path \
    && IFS= read -r -d '' kind \
    && IFS= read -r -d '' file_mode \
    && IFS= read -r -d '' content_sha; do
    [[ -n "$path" && "$path" != /* && "$path" != "$previous" ]] || reject 'repository manifest contains an empty, absolute, or duplicate adjacent path'
    [[ -z "$previous" || "$previous" < "$path" ]] || reject 'repository manifest paths are not strictly byte-sorted'
    case "$kind" in regular|symlink|missing|gitlink) ;; *) reject "repository manifest kind is invalid for $path" ;; esac
    [[ "$file_mode" == missing || "$file_mode" =~ ^[0-9]{3,6}$ ]] || reject "repository manifest mode is invalid for $path"
    is_hex "$content_sha" || reject "repository manifest content digest is invalid for $path"
    entry_sha="$(printf '%s\0%s\0%s\0%s\0' "$path" "$kind" "$file_mode" "$content_sha" | sha256_stream)"
    printf '%s\0%s\0%s\0%s\0' "repo:$path" "repository_snapshot/$kind" "$path" "$entry_sha"
    previous="$path"
  done <"$repo_root/$REPOSITORY_MANIFEST_REL"
}

step_input_digest() {
  local fv="$1" state="$2" ignored_rows="$3" dispatch_json="$4" protocol_rows="$5"
  local input_id classification subkind path byte_sha
  {
    printf '%s\0' "$fv"
    printf '%s\0%s\0%s\0%s\0' \
      repository-manifest-component repository_snapshot/component "$REPOSITORY_MANIFEST_REL" \
      "$(jq -r '.repository_manifest_sha256' <<<"$state")"
    repository_tuple_stream
    while IFS=$'\t' read -r input_id classification subkind path byte_sha; do
      printf '%s\0%s\0%s\0%s\0' "$input_id" "$classification/$subkind" "$path" "$byte_sha"
    done < <(jq -r '.[] | [.input_id,.classification,.role,.path,.sha256] | @tsv' <<<"$ignored_rows")
    while IFS=$'\t' read -r input_id path byte_sha; do
      printf '%s\0%s\0%s\0%s\0' "$input_id" immutable_protocol_output/generated_protocol_evidence "$path" "$byte_sha"
    done < <(jq -r '.gpe_artifact_hash_map[] | [.input_id,.path,.byte_sha256] | @tsv' <<<"$state")
    printf '%s\0%s\0%s\0%s\0' gpe-artifact-hash-map immutable_protocol_output/generated_protocol_evidence-map \
      metadata:gpe_artifact_hash_map "$(jq -r '.gpe_artifact_hash_map_sha256' <<<"$state")"
    printf '%s\0%s\0%s\0%s\0' \
      "$(jq -r '.input_id' <<<"$dispatch_json")" repository_snapshot/dispatch_descriptor \
      "$(jq -r '.owning_tracked_path' <<<"$dispatch_json")" "$(jq -r '.descriptor_sha256' <<<"$dispatch_json")"
    while IFS=$'\t' read -r input_id classification subkind path byte_sha; do
      printf '%s\0%s\0%s\0%s\0' "$input_id" "$classification/$subkind" "$path" "$byte_sha"
    done < <(jq -r '.[] | [.input_id,.classification,.subkind,.path,.sha256] | @tsv' <<<"$protocol_rows")
  } | sha256_stream
}

verify_snapshot() {
  local fv="$1" expected_map="${TASK13_R25_GPE_MAP_SHA256:-}" state input_spec dispatch_json
  local repo_ids ignored_rows protocol_rows step_digest input_id protocol_sha classification subkind protocol_path
  case "$fv" in FV01|FV02|FV03|FV04|FV05|FV06|FV07|FV08) ;; *) reject 'final-tree verify-snapshot accepts uppercase FV01 through FV08 only' ;; esac
  is_hex "$expected_map" || reject 'verified GPE-map environment receipt is missing'
  validate_catalog
  state="$(require_current_snapshot "$expected_map")"
  input_spec="$(jq -ce --arg fv "$fv" '.final_tree_record_protocol.fv_input_sets[$fv]' "$manifest")" || reject "manifest FV input set is missing for $fv"
  jq -e 'keys | sort == (["descriptor_dispatch_input_ids","expanded_repository_snapshot_input_ids","explicit_ignored_evidence_input_ids","generated_protocol_evidence_hash_map_ref","generated_protocol_evidence_input_ids","immutable_protocol_output_input_ids","repository_manifest_component_hash","repository_snapshot_selection"] | sort)' <<<"$input_spec" >/dev/null || reject "manifest FV input set schema differs for $fv"
  [[ "$(jq -r '.repository_snapshot_selection' <<<"$input_spec")" == all ]] || reject 'FV repository snapshot selection is not all'
  repo_ids="$(repository_ids_json)"
  ignored_rows="$(jq -cS --argjson ids "$(jq -c '.explicit_ignored_evidence_input_ids' <<<"$input_spec")" '
    [$ids[] as $id
      | ([.entries[] | select(.input_id == $id)]
         | if length == 1 then .[0] else error("unresolved ignored input") end)]
  ' "$repo_root/$IGNORED_INVENTORY_REL")" || reject "FV ignored inputs are unresolved for $fv"
  jq -e 'all(.[];
    (keys|sort) == (["classification","input_id","path","producer","role","sha256"]|sort)
    and .classification == "explicit_ignored_evidence"
    and (.input_id|type=="string" and length>0)
    and (.path|type=="string" and length>0)
    and (.role|type=="string" and length>0)
    and (.sha256|test("^[0-9a-f]{64}$")))' <<<"$ignored_rows" >/dev/null || reject "FV ignored input rows are malformed for $fv"
  protocol_rows='[]'
  while IFS= read -r input_id; do
    [[ -n "$input_id" ]] || continue
    protocol_sha="$(catalog_output_hash "$input_id")" || reject "FV protocol input hash is invalid for $input_id"
    is_hex "$protocol_sha" || reject "FV protocol input digest is malformed for $input_id"
    classification="$(jq -er --arg id "$input_id" '.[] | select(.input_id == $id) | .classification' "$repo_root/$CATALOG_REL")"
    subkind="$(jq -er --arg id "$input_id" '.[] | select(.input_id == $id) | .subkind' "$repo_root/$CATALOG_REL")"
    protocol_path="$(jq -er --arg id "$input_id" '.[] | select(.input_id == $id) | .path_or_external_selector' "$repo_root/$CATALOG_REL")"
    protocol_rows="$(jq -cnS --argjson rows "$protocol_rows" --arg id "$input_id" --arg sha "$protocol_sha" \
      --arg classification "$classification" --arg subkind "$subkind" --arg path "$protocol_path" \
      '$rows + [{classification:$classification,input_id:$id,path:$path,sha256:$sha,subkind:$subkind}]')"
  done < <(jq -r '.immutable_protocol_output_input_ids[]' <<<"$input_spec")
  dispatch_json="$(resolved_dispatch_descriptor "$fv")"
  step_digest="$(step_input_digest "$fv" "$state" "$ignored_rows" "$dispatch_json" "$protocol_rows")"
  jq -cnS --arg fv_id "$fv" --arg step_input_digest "$step_digest" --arg source_snapshot "$(jq -r '.source_snapshot' <<<"$state")" \
    --arg gpe_artifact_hash_map_sha256 "$(jq -r '.gpe_artifact_hash_map_sha256' <<<"$state")" \
    --argjson gpe_artifact_hash_map "$(jq -c '.gpe_artifact_hash_map' <<<"$state")" \
    --arg repository_manifest_sha256 "$(jq -r '.repository_manifest_sha256' <<<"$state")" \
    --arg ignored_input_inventory_sha256 "$(jq -r '.ignored_input_inventory_sha256' <<<"$state")" \
    --arg input_coverage_oracle_sha256 "$(jq -r '.input_coverage_oracle_sha256' <<<"$state")" \
    --arg source_snapshot_hash_artifact_sha256 "$(jq -r '.source_snapshot_hash_artifact_sha256' <<<"$state")" \
    --arg generated_evidence_binding_sha256 "$(jq -r '.generated_evidence_binding_sha256' <<<"$state")" \
    --argjson expanded_repository_snapshot_input_ids "$repo_ids" --argjson explicit_ignored_inputs "$ignored_rows" \
    --argjson immutable_protocol_inputs "$protocol_rows" --argjson dispatch_descriptor "$dispatch_json" \
    '{dispatch_descriptor:$dispatch_descriptor,expanded_repository_snapshot_input_ids:$expanded_repository_snapshot_input_ids,explicit_ignored_inputs:$explicit_ignored_inputs,fv_id:$fv_id,generated_evidence_binding_sha256:$generated_evidence_binding_sha256,gpe_artifact_hash_map:$gpe_artifact_hash_map,gpe_artifact_hash_map_sha256:$gpe_artifact_hash_map_sha256,ignored_input_inventory_sha256:$ignored_input_inventory_sha256,immutable_protocol_inputs:$immutable_protocol_inputs,input_coverage_oracle_sha256:$input_coverage_oracle_sha256,repository_manifest_sha256:$repository_manifest_sha256,source_snapshot:$source_snapshot,source_snapshot_hash_artifact_sha256:$source_snapshot_hash_artifact_sha256,step_input_digest:$step_input_digest}'
}

descriptor_digest() {
  local source_snapshot="$1" map_sha="$2" target_id="$3" pair_identity="$4" action_identity="$5" capability_slot="$6" output_schema_sha="$7"
  printf '%s\0%s\0%s\0%s\0%s\0%s\0%s\0%s\0%s\0' \
    "$GENERATION" "$assertion_digest" "$source_snapshot" "$map_sha" "$target_id" \
    "$pair_identity" "$action_identity" "$capability_slot" "$output_schema_sha" | sha256_stream
}

validate_output_path() {
  local path="$1"
  [[ "$path" =~ ^/nix/store/[0-9abcdfghijklmnpqrsvwxyz]{32}-[-+._?=A-Za-z0-9]+$ \
    && -e "$path" && ! -L "$path" ]] || return 1
}

json_payload_hash() {
  local path="$1" digest_field="$2"
  jq -cS "del(.$digest_field)" "$path" | sha256_stream
}

validate_terminal() {
  local path="$1" system="$2" expected_target="$3" expected_descriptor="$4" expected_map="$5" expected_source="$6" displayed payload terminal_map
  require_canonical_object "$path" "descriptor terminal $system"
  jq -e '
    (keys | sort) == (["session","assertion_digest","source_snapshot","gpe_artifact_hash_map","gpe_artifact_hash_map_sha256","target_descriptor_id","descriptor_digest","capability_evidence","reservation_id","terminal_state","terminal_failure_class","original_invocation_count","invocation_id","command_identity","start_timestamp","end_timestamp","raw_exit","raw_evidence_reference","post_action_source_snapshot","post_action_ignored_inventory_sha256","post_action_generated_evidence_binding_sha256","post_action_gpe_artifact_hash_map","post_action_gpe_artifact_hash_map_sha256","post_action_descriptor_digest","attribute_to_output_path","record_sha256"] | sort)
    and (.terminal_state == "build_succeeded" or .terminal_state == "build_failed" or .terminal_state == "input_drift")
    and .original_invocation_count == 1
  ' "$path" >/dev/null || reject "descriptor terminal schema is invalid for $system"
  [[ "$(jq -er '.session' "$path")" == "$GENERATION" \
    && "$(jq -er '.assertion_digest' "$path")" == "$assertion_digest" \
    && "$(jq -er '.source_snapshot' "$path")" == "$expected_source" \
    && "$(jq -er '.gpe_artifact_hash_map_sha256' "$path")" == "$expected_map" \
    && "$(jq -er '.target_descriptor_id' "$path")" == "$expected_target" \
    && "$(jq -er '.descriptor_digest' "$path")" == "$expected_descriptor" ]] || reject "descriptor terminal key differs for $system"
  terminal_map="$(jq -cS '.gpe_artifact_hash_map' "$path")"
  [[ "$(gpe_map_sha "$terminal_map")" == "$expected_map" ]] || reject "descriptor terminal GPE map payload differs for $system"
  displayed="$(jq -er '.record_sha256 | select(test("^[0-9a-f]{64}$"))' "$path")" || reject "terminal record hash is malformed for $system"
  payload="$(json_payload_hash "$path" record_sha256)"
  [[ "$displayed" == "$payload" ]] || reject "terminal payload hash differs for $system"
  case "$system" in
    aarch64-linux)
      jq -e --arg source "$expected_source" '.capability_evidence | (keys | sort) == ["command","exit","source_snapshot","stderr_sha256","stdout_sha256","target_system","verdict"]
        and .command == ["nix","build","--dry-run","--no-link","--print-out-paths","--no-write-lock-file","path:.#packages.aarch64-linux.api","path:.#packages.aarch64-linux.worker"]
        and .exit == 0 and .source_snapshot == $source
        and (.stderr_sha256|test("^[0-9a-f]{64}$")) and (.stdout_sha256|test("^[0-9a-f]{64}$"))
        and .target_system == "aarch64-linux" and .verdict == "capable"' "$path" >/dev/null || reject 'aarch64-linux terminal capability evidence is malformed' ;;
    *) jq -e '.capability_evidence == null' "$path" >/dev/null || reject "$system terminal unexpectedly contains capability evidence" ;;
  esac
  if [[ "$(jq -r '.terminal_state' "$path")" == build_succeeded ]]; then
    jq -e --arg source "$expected_source" --arg map "$expected_map" --arg descriptor "$expected_descriptor" '
      .terminal_failure_class == null and .raw_exit == 0
      and .post_action_source_snapshot == $source
      and .post_action_gpe_artifact_hash_map_sha256 == $map
      and .post_action_descriptor_digest == $descriptor
      and (.post_action_ignored_inventory_sha256|test("^[0-9a-f]{64}$"))
      and (.post_action_generated_evidence_binding_sha256|test("^[0-9a-f]{64}$"))
      and (.attribute_to_output_path | type == "object" and length == 2)
    ' "$path" >/dev/null || reject "$system successful terminal has inconsistent post-action state"
    [[ "$(jq -cS '.post_action_gpe_artifact_hash_map' "$path")" == "$terminal_map" ]] || reject "$system successful terminal post-action GPE map differs"
    case "$system" in
      x86_64-linux) jq -e '(.attribute_to_output_path|keys) == ["packages.x86_64-linux.api","packages.x86_64-linux.worker"]' "$path" >/dev/null ;;
      aarch64-linux) jq -e '(.attribute_to_output_path|keys) == ["packages.aarch64-linux.api","packages.aarch64-linux.worker"]' "$path" >/dev/null ;;
      aarch64-darwin) jq -e '(.attribute_to_output_path|keys) == ["packages.aarch64-darwin.api","packages.aarch64-darwin.worker"]' "$path" >/dev/null ;;
    esac || reject "$system successful terminal output attributes differ from the exact schema"
    while IFS= read -r output; do validate_output_path "$output" || reject "$system successful terminal has an invalid store output"; done < <(jq -r '.attribute_to_output_path[]' "$path")
  else
    jq -e '.attribute_to_output_path == null and .terminal_failure_class != null' "$path" >/dev/null || reject "$system failed terminal contains a success output map"
  fi
}

validate_receipt() {
  local path="$1" expected_kind="$2" terminal="$3" descriptor="$4" map_sha="$5" source_sha="$6" expected_current="$7" displayed payload
  require_canonical_object "$path" "descriptor $expected_kind receipt"
  jq -e '
    (keys | sort) == (["session","assertion_digest","source_snapshot","gpe_artifact_hash_map","gpe_artifact_hash_map_sha256","target_descriptor_id","descriptor_digest","record_path","record_sha256","reservation_id","call_kind","current_call_invocation_count","returned_terminal_state","returned_exit","receipt_sha256"] | sort)
  ' "$path" >/dev/null || reject "descriptor $expected_kind receipt schema is invalid"
  [[ "$(jq -er '.session' "$path")" == "$GENERATION" \
    && "$(jq -er '.assertion_digest' "$path")" == "$assertion_digest" \
    && "$(jq -er '.source_snapshot' "$path")" == "$source_sha" \
    && "$(jq -er '.gpe_artifact_hash_map_sha256' "$path")" == "$map_sha" \
    && "$(jq -er '.descriptor_digest' "$path")" == "$descriptor" \
    && "$(jq -er '.record_path' "$path")" == "$terminal" \
    && "$(jq -er '.call_kind' "$path")" == "$expected_kind" \
    && "$(jq -er '.current_call_invocation_count' "$path")" == "$expected_current" ]] || reject "descriptor $expected_kind receipt binding differs"
  [[ "$(jq -r '.target_descriptor_id' "$path")" == "$(jq -r '.target_descriptor_id' "$repo_root/$terminal")" \
    && "$(jq -r '.record_sha256' "$path")" == "$(jq -r '.record_sha256' "$repo_root/$terminal")" \
    && "$(jq -r '.reservation_id' "$path")" == "$(jq -r '.reservation_id' "$repo_root/$terminal")" \
    && "$(jq -cS '.gpe_artifact_hash_map' "$path")" == "$(jq -cS '.gpe_artifact_hash_map' "$repo_root/$terminal")" \
    && "$(jq -r '.returned_terminal_state' "$path")" == "$(jq -r '.terminal_state' "$repo_root/$terminal")" ]] || reject "descriptor $expected_kind receipt does not bind its terminal"
  displayed="$(jq -er '.receipt_sha256 | select(test("^[0-9a-f]{64}$"))' "$path")" || reject "receipt hash is malformed for $expected_kind"
  payload="$(json_payload_hash "$path" receipt_sha256)"
  [[ "$displayed" == "$payload" ]] || reject "receipt payload hash differs for $expected_kind"
}

publish_hashed_json() {
  local payload_json="$1" digest_field="$2" destination="$3" staged="$4" payload_sha
  [[ ! -e "$staged" && ! -L "$staged" ]] || reject "staged $digest_field output already exists"
  payload_sha="$(jq -cS '.' <<<"$payload_json" | sha256_stream)"
  jq -cnS --argjson payload "$payload_json" --arg sha "$payload_sha" --arg field "$digest_field" '$payload + {($field):$sha}' >"$staged"
  chmod 0400 -- "$staged"
  exclusive_publish "$staged" "$destination" "$(basename -- "$destination")"
  require_canonical_object "$destination" "$(basename -- "$destination")"
  [[ "$(json_payload_hash "$destination" "$digest_field")" == "$payload_sha" ]] || reject "published $digest_field payload hash differs"
  printf '%s' "$payload_sha"
}

descriptor_target_values() {
  local system="$1"
  jq -cer --arg system "$system" '.final_tree_record_protocol.descriptor_protocol.targets[$system]' "$manifest"
}

validate_descriptor_contract() {
  local system="$1"
  case "$system" in
    x86_64-linux)
      jq -e '
        .final_tree_record_protocol.descriptor_protocol.output_map_schema["x86_64-linux"] == {ordered_attributes:["packages.x86_64-linux.api","packages.x86_64-linux.worker"],sha256:"b1255e5e83efb6751da8d1a0ff50cc5aa19df2faad6f24b50f1b30947958a8c7"}
        and .final_tree_record_protocol.descriptor_protocol.targets["x86_64-linux"].target_descriptor_id == "x86_64-linux-flake-linux-api-worker"
        and .final_tree_record_protocol.descriptor_protocol.targets["x86_64-linux"].pair_identity == "scripts/tasks/task-13/r25-shape.just::linux-builder-r25-exec"
        and .final_tree_record_protocol.descriptor_protocol.targets["x86_64-linux"].internal_action_identity == "strict-pair:scripts/tasks/task-1/mod.just::flake-linux"
        and .final_tree_record_protocol.descriptor_protocol.targets["x86_64-linux"].capability_slot == "none"
      ' "$manifest" >/dev/null ;;
    aarch64-linux)
      jq -e '
        .final_tree_record_protocol.descriptor_protocol.output_map_schema["aarch64-linux"] == {ordered_attributes:["packages.aarch64-linux.api","packages.aarch64-linux.worker"],sha256:"e7769ba4d9a8d4c7dc5ef72ae0ddf66695cca2a57fb6a33f31cc71a7d7287de1"}
        and .final_tree_record_protocol.descriptor_protocol.targets["aarch64-linux"].target_descriptor_id == "aarch64-linux-api-worker"
        and .final_tree_record_protocol.descriptor_protocol.targets["aarch64-linux"].pair_identity == "scripts/tasks/task-13/r25-shape.just::aarch64-linux-builder-r25-exec"
        and .final_tree_record_protocol.descriptor_protocol.targets["aarch64-linux"].internal_action_identity == "nix build --no-link --print-out-paths --no-write-lock-file path:.#packages.aarch64-linux.api path:.#packages.aarch64-linux.worker"
        and .final_tree_record_protocol.descriptor_protocol.targets["aarch64-linux"].capability_command == ["nix","build","--dry-run","--no-link","--print-out-paths","--no-write-lock-file","path:.#packages.aarch64-linux.api","path:.#packages.aarch64-linux.worker"]
        and .final_tree_record_protocol.descriptor_protocol.targets["aarch64-linux"].action_command == ["nix","build","--no-link","--print-out-paths","--no-write-lock-file","path:.#packages.aarch64-linux.api","path:.#packages.aarch64-linux.worker"]
      ' "$manifest" >/dev/null ;;
    aarch64-darwin)
      jq -e '
        .final_tree_record_protocol.descriptor_protocol.output_map_schema["aarch64-darwin"] == {ordered_attributes:["packages.aarch64-darwin.api","packages.aarch64-darwin.worker"],sha256:"9e47eaae19a001012ae422c76becf1eb9e486df657352a2f668d3be024a5f94f"}
        and .final_tree_record_protocol.descriptor_protocol.targets["aarch64-darwin"].target_descriptor_id == "aarch64-darwin-api-worker"
        and .final_tree_record_protocol.descriptor_protocol.targets["aarch64-darwin"].pair_identity == "scripts/tasks/task-13/r25-shape.just::cross-system-verify-r25-exec"
        and .final_tree_record_protocol.descriptor_protocol.targets["aarch64-darwin"].internal_action_identity == "nix build --no-link --print-out-paths --no-write-lock-file path:.#packages.aarch64-darwin.api path:.#packages.aarch64-darwin.worker"
        and .final_tree_record_protocol.descriptor_protocol.targets["aarch64-darwin"].action_command == ["nix","build","--no-link","--print-out-paths","--no-write-lock-file","path:.#packages.aarch64-darwin.api","path:.#packages.aarch64-darwin.worker"]
        and .final_tree_record_protocol.descriptor_protocol.targets["aarch64-darwin"].capability_slot == "none"
      ' "$manifest" >/dev/null ;;
  esac || reject "$system descriptor contract differs from frozen R25 authority"
}

descriptor_paths() {
  local system="$1"
  jq -cer --arg system "$system" '
    .final_tree_record_protocol.descriptor_protocol.targets[$system]
    | {terminal_path,first_receipt_path,replay_receipt_path}
  ' "$manifest"
}

require_darwin_prerequisites() {
  local map_sha="$1" source_sha="$2" system values paths terminal first target descriptor capability_slot output_sha
  for system in x86_64-linux aarch64-linux; do
    values="$(descriptor_target_values "$system")"
    paths="$(descriptor_paths "$system")"
    terminal="$(jq -r '.terminal_path' <<<"$paths")"
    first="$(jq -r '.first_receipt_path' <<<"$paths")"
    target="$(jq -r '.target_descriptor_id' <<<"$values")"
    if [[ "$system" == aarch64-linux ]]; then
      require_regular_nonlink "$repo_root/$terminal" 'aarch64-linux prerequisite terminal'
      capability_slot="$(jq -cS '.capability_evidence' "$repo_root/$terminal" | sha256_stream)"
    else capability_slot=none; fi
    output_sha="$(jq -r --arg system "$system" '.final_tree_record_protocol.descriptor_protocol.output_map_schema[$system].sha256' "$manifest")"
    descriptor="$(descriptor_digest "$source_sha" "$map_sha" "$target" "$(jq -r '.pair_identity' <<<"$values")" "$(jq -r '.internal_action_identity' <<<"$values")" "$capability_slot" "$output_sha")"
    validate_terminal "$repo_root/$terminal" "$system" "$target" "$descriptor" "$map_sha" "$source_sha"
    [[ "$(jq -r '.terminal_state' "$repo_root/$terminal")" == build_succeeded ]] || reject "$system prerequisite terminal did not succeed"
    validate_receipt "$repo_root/$first" first "$terminal" "$descriptor" "$map_sha" "$source_sha" 1
    [[ "$(jq -r '.returned_terminal_state' "$repo_root/$first")" == build_succeeded && "$(jq -r '.returned_exit' "$repo_root/$first")" == 0 ]] || reject "$system prerequisite receipt did not return success"
  done
}

run_descriptor_replay() {
  local system="$1" map_sha="$2" state="$3" values="$4" paths="$5" terminal first replay
  local target pair action output_sha capability_json capability_slot descriptor record_sha reservation_id receipt_payload receipt_sha reservation_rel
  terminal="$(jq -r '.terminal_path' <<<"$paths")"
  first="$(jq -r '.first_receipt_path' <<<"$paths")"
  replay="$(jq -r '.replay_receipt_path' <<<"$paths")"
  require_regular_nonlink "$repo_root/$terminal" "$system terminal"
  [[ ! -e "$repo_root/$replay" && ! -L "$repo_root/$replay" ]] || reject "$system FV07 replay receipt already exists"
  target="$(jq -r '.target_descriptor_id' <<<"$values")"
  pair="$(jq -r '.pair_identity' <<<"$values")"
  action="$(jq -r '.internal_action_identity' <<<"$values")"
  output_sha="$(jq -r --arg system "$system" '.final_tree_record_protocol.descriptor_protocol.output_map_schema[$system].sha256' "$manifest")"
  if [[ "$system" == aarch64-linux ]]; then
    capability_json="$(jq -cS '.capability_evidence' "$repo_root/$terminal")"
    capability_slot="$(printf '%s\n' "$capability_json" | sha256_stream)"
  else
    capability_json=null
    capability_slot=none
  fi
  descriptor="$(descriptor_digest "$(jq -r '.source_snapshot' <<<"$state")" "$map_sha" "$target" "$pair" "$action" "$capability_slot" "$output_sha")"
  validate_terminal "$repo_root/$terminal" "$system" "$target" "$descriptor" "$map_sha" "$(jq -r '.source_snapshot' <<<"$state")"
  [[ "$(jq -r '.terminal_state' "$repo_root/$terminal")" == build_succeeded ]] || reject "$system failed terminal is not replay eligible"
  validate_receipt "$repo_root/$first" first "$terminal" "$descriptor" "$map_sha" "$(jq -r '.source_snapshot' <<<"$state")" 1
  record_sha="$(jq -r '.record_sha256' "$repo_root/$terminal")"
  reservation_id="$(jq -r '.reservation_id' "$repo_root/$terminal")"
  reservation_rel="$(jq -er --arg system "$system" '.final_tree_record_protocol.reservation_abi.descriptor_paths[$system]' "$manifest")"
  [[ -d "$repo_root/$reservation_rel" && ! -L "$repo_root/$reservation_rel" ]] || reject "$system descriptor reservation is missing for replay"
  receipt_payload="$(jq -cnS --arg session "$GENERATION" --arg assertion_digest "$assertion_digest" \
    --arg source_snapshot "$(jq -r '.source_snapshot' <<<"$state")" --argjson map "$(jq -c '.gpe_artifact_hash_map' <<<"$state")" \
    --arg map_sha "$map_sha" --arg target "$target" --arg descriptor "$descriptor" --arg terminal "$terminal" \
    --arg record_sha "$record_sha" --arg reservation_id "$reservation_id" '
    {assertion_digest:$assertion_digest,call_kind:"fv07_replay",current_call_invocation_count:0,descriptor_digest:$descriptor,gpe_artifact_hash_map:$map,gpe_artifact_hash_map_sha256:$map_sha,record_path:$terminal,record_sha256:$record_sha,reservation_id:$reservation_id,returned_exit:0,returned_terminal_state:"build_succeeded",session:$session,source_snapshot:$source_snapshot,target_descriptor_id:$target}' )"
  receipt_sha="$(publish_hashed_json "$receipt_payload" receipt_sha256 "$repo_root/$replay" "$repo_root/$reservation_rel/fv07-replay-receipt.json")"
  jq -cnS --arg descriptor_digest "$descriptor" --arg receipt_path "$replay" --arg receipt_sha256 "$receipt_sha" --arg target_descriptor_id "$target" '{descriptor_digest:$descriptor_digest,receipt_path:$receipt_path,receipt_sha256:$receipt_sha256,target_descriptor_id:$target_descriptor_id,terminal_state:"build_succeeded"}'
}

run_capability_preflight() {
  local source_sha="$1" expected_state="$2" directory stdout_file stderr_file status stdout_sha stderr_sha object
  directory="$(mktemp -d "${TMPDIR:-/tmp}/jamye-task13-r25-capability.XXXXXX")" || reject 'cannot stage aarch64-linux capability evidence'
  stdout_file="$directory/stdout"; stderr_file="$directory/stderr"
  set +e
  nix build --dry-run --no-link --print-out-paths --no-write-lock-file \
    path:.#packages.aarch64-linux.api path:.#packages.aarch64-linux.worker >"$stdout_file" 2>"$stderr_file"
  status=$?
  set -e
  stdout_sha="$(sha256_file "$stdout_file")"; stderr_sha="$(sha256_file "$stderr_file")"
  object="$(jq -cnS --arg source "$source_sha" --arg stdout "$stdout_sha" --arg stderr "$stderr_sha" --argjson exit "$status" '
    {command:["nix","build","--dry-run","--no-link","--print-out-paths","--no-write-lock-file","path:.#packages.aarch64-linux.api","path:.#packages.aarch64-linux.worker"],exit:$exit,source_snapshot:$source,stderr_sha256:$stderr,stdout_sha256:$stdout,target_system:"aarch64-linux",verdict:(if $exit == 0 then "capable" else "incapable" end)}')"
  rm -rf -- "$directory"
  (( status == 0 )) || reject 'aarch64-linux capability preflight failed before reservation'
  [[ "$(require_current_snapshot "${TASK13_R25_GPE_MAP_SHA256}")" == "$expected_state" ]] || reject 'source/GPE state drifted across aarch64-linux capability preflight'
  printf '%s\n' "$object"
}

parse_action_outputs() {
  local system="$1" stdout_file="$2" digest="$3" api_path worker_path line_count
  [[ "$(tail -c 1 "$stdout_file" | od -An -tuC | tr -d ' ')" == 10 ]] || return 1
  line_count="$(wc -l <"$stdout_file" | tr -d ' ')"
  if [[ "$system" == x86_64-linux ]]; then
    [[ "$line_count" == 7 ]] || return 1
    mapfile -t lines <"$stdout_file"
    [[ "${lines[0]}" == "task13_assertion_manifest_sha256=$digest" \
      && "${lines[1]}" == "task13_assertion_generation=$GENERATION" \
      && "${lines[2]}" == 'This card requires an x86_64-linux builder. Failure to find one is an M0 blocker, not a skip.' \
      && "${lines[3]}" == 'built x86_64-linux package outputs:' \
      && "${lines[6]}" == 'x86_64-linux API and worker package build passed' ]] || return 1
    api_path="${lines[4]}"; worker_path="${lines[5]}"
  else
    [[ "$line_count" == 2 ]] || return 1
    mapfile -t lines <"$stdout_file"
    api_path="${lines[0]}"; worker_path="${lines[1]}"
  fi
  [[ "$api_path" != "$worker_path" ]] || return 1
  validate_output_path "$api_path" && validate_output_path "$worker_path" || return 1
  jq -cnS --arg system "$system" --arg api "$api_path" --arg worker "$worker_path" '{("packages."+$system+".api"):$api,("packages."+$system+".worker"):$worker}'
}

run_descriptor_first() {
  local system="$1" map_sha="$2" state="$3" values="$4" paths="$5"
  local target pair action output_sha capability_json capability_slot descriptor reservation_rel reservation reservation_id
  local terminal first replay stdout_file stderr_file start_timestamp end_timestamp raw_exit output_map=null parsed post_state post_status
  local terminal_state terminal_failure returned_exit post_source='' post_ignored='' post_binding='' post_map=null post_map_sha='' post_descriptor='' output_valid=false
  local terminal_payload record_sha receipt_payload receipt_sha invocation_id raw_reference command_identity
  target="$(jq -r '.target_descriptor_id' <<<"$values")"
  pair="$(jq -r '.pair_identity' <<<"$values")"
  action="$(jq -r '.internal_action_identity' <<<"$values")"
  output_sha="$(jq -r --arg system "$system" '.final_tree_record_protocol.descriptor_protocol.output_map_schema[$system].sha256' "$manifest")"
  terminal="$(jq -r '.terminal_path' <<<"$paths")"
  first="$(jq -r '.first_receipt_path' <<<"$paths")"
  replay="$(jq -r '.replay_receipt_path' <<<"$paths")"
  for path in "$terminal" "$first" "$replay"; do [[ ! -e "$repo_root/$path" && ! -L "$repo_root/$path" ]] || reject "$system descriptor output already exists"; done

  if [[ "$system" == aarch64-linux ]]; then
    # The exact capability command and its post-preflight equality check occur
    # before any reservation.  Replay never enters this branch.
    capability_json="$(run_capability_preflight "$(jq -r '.source_snapshot' <<<"$state")" "$state")"
    capability_slot="$(printf '%s\n' "$capability_json" | sha256_stream)"
  else
    capability_json=null
    capability_slot=none
  fi
  descriptor="$(descriptor_digest "$(jq -r '.source_snapshot' <<<"$state")" "$map_sha" "$target" "$pair" "$action" "$capability_slot" "$output_sha")"
  if [[ "$system" == aarch64-darwin ]]; then require_darwin_prerequisites "$map_sha" "$(jq -r '.source_snapshot' <<<"$state")"; fi
  reservation_rel="$(jq -er --arg system "$system" '.final_tree_record_protocol.reservation_abi.descriptor_paths[$system]' "$manifest")"
  [[ "$reservation_rel" == ".agents/results/.task-13-r25-${system}-reservation-20260907" ]] || reject "$system reservation path differs from R25 ABI"
  reservation="$repo_root/$reservation_rel"
  reservation_id="$(printf '%s\0%s\0%s\0%s\0%s\0%s\0' "$GENERATION" "$assertion_digest" "$(jq -r '.source_snapshot' <<<"$state")" "$map_sha" "$target" "$descriptor" | sha256_stream)"
  mkdir -- "$reservation" || reject "$system descriptor reservation already exists or cannot be acquired"
  jq -cnS --arg session "$GENERATION" --arg assertion_digest "$assertion_digest" --arg source_snapshot "$(jq -r '.source_snapshot' <<<"$state")" \
    --argjson map "$(jq -c '.gpe_artifact_hash_map' <<<"$state")" --arg map_sha "$map_sha" --arg target "$target" --arg descriptor "$descriptor" \
    --arg reservation_id "$reservation_id" --arg timestamp "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --arg path "$reservation_rel" \
    '{assertion_digest:$assertion_digest,descriptor_digest:$descriptor,gpe_artifact_hash_map:$map,gpe_artifact_hash_map_sha256:$map_sha,reservation_id:$reservation_id,reservation_path:$path,reservation_timestamp:$timestamp,session:$session,source_snapshot:$source_snapshot,state:"reserved",target_descriptor_id:$target}' >"$reservation/reservation.json"
  chmod 0400 -- "$reservation/reservation.json"

  stdout_file="$reservation/action.stdout"; stderr_file="$reservation/action.stderr"
  start_timestamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  set +e
  case "$system" in
    x86_64-linux)
      TASK13_R25_ENTRYPOINT_SHA256="${TASK13_R25_STRICT_ENTRYPOINT_SHA256}" \
        bash "${TASK13_R25_STRICT_ENTRYPOINT_COPY}" "$assertion_digest" pair scripts/tasks/task-1/mod.just flake-linux >"$stdout_file" 2>"$stderr_file" ;;
    aarch64-linux)
      nix build --no-link --print-out-paths --no-write-lock-file \
        path:.#packages.aarch64-linux.api path:.#packages.aarch64-linux.worker >"$stdout_file" 2>"$stderr_file" ;;
    aarch64-darwin)
      nix build --no-link --print-out-paths --no-write-lock-file \
        path:.#packages.aarch64-darwin.api path:.#packages.aarch64-darwin.worker >"$stdout_file" 2>"$stderr_file" ;;
  esac
  raw_exit=$?
  set -e
  end_timestamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  chmod 0400 -- "$stdout_file" "$stderr_file"
  if (( raw_exit == 0 )); then
    if parsed="$(parse_action_outputs "$system" "$stdout_file" "$assertion_digest")"; then output_map="$parsed"; output_valid=true; fi
  fi

  set +e
  post_state="$(snapshot_state 2>"$reservation/post-action-verification.stderr")"
  post_status=$?
  set -e
  chmod 0400 -- "$reservation/post-action-verification.stderr"
  if (( post_status == 0 )); then
    post_source="$(jq -r '.current_source_snapshot' <<<"$post_state")"
    post_ignored="$(jq -r '.current_ignored_input_inventory_sha256' <<<"$post_state")"
    post_binding="$(jq -r '.current_generated_evidence_binding_sha256' <<<"$post_state")"
    post_map="$(jq -cS '.gpe_artifact_hash_map' <<<"$post_state")"
    post_map_sha="$(jq -r '.gpe_artifact_hash_map_sha256' <<<"$post_state")"
    post_descriptor="$(descriptor_digest "$post_source" "$post_map_sha" "$target" "$pair" "$action" "$capability_slot" "$output_sha")"
  fi
  if (( post_status != 0 )) \
    || [[ "$(jq -r '.current_equal' <<<"$post_state")" != true \
      || "$post_source" != "$(jq -r '.source_snapshot' <<<"$state")" \
      || "$post_map_sha" != "$map_sha" \
      || "$post_descriptor" != "$descriptor" ]]; then
    terminal_state=input_drift; terminal_failure=input_drift; output_map=null; returned_exit=1
  elif (( raw_exit != 0 )); then
    terminal_state=build_failed; terminal_failure=action_failed; output_map=null; returned_exit="$raw_exit"
  elif [[ "$output_valid" != true ]]; then
    terminal_state=build_failed; terminal_failure=output_schema_invalid; output_map=null; returned_exit=1
  else
    terminal_state=build_succeeded; terminal_failure=''; returned_exit=0
  fi
  invocation_id="$(printf '%s\0%s\0%s\0' "$descriptor" first "$reservation_id" | sha256_stream)"
  raw_reference="$(jq -cnS --arg stdout "$reservation_rel/action.stdout" --arg stderr "$reservation_rel/action.stderr" \
    --arg stdout_sha "$(sha256_file "$stdout_file")" --arg stderr_sha "$(sha256_file "$stderr_file")" \
    '{stderr_path:$stderr,stderr_sha256:$stderr_sha,stdout_path:$stdout,stdout_sha256:$stdout_sha}')"
  command_identity="$action"
  terminal_payload="$(jq -cnS --arg session "$GENERATION" --arg assertion_digest "$assertion_digest" \
    --arg source_snapshot "$(jq -r '.source_snapshot' <<<"$state")" --argjson map "$(jq -c '.gpe_artifact_hash_map' <<<"$state")" \
    --arg map_sha "$map_sha" --arg target "$target" --arg descriptor "$descriptor" --argjson capability "$capability_json" \
    --arg reservation_id "$reservation_id" --arg terminal_state "$terminal_state" --arg terminal_failure "$terminal_failure" \
    --arg invocation_id "$invocation_id" --arg command_identity "$command_identity" --arg start "$start_timestamp" --arg end "$end_timestamp" \
    --argjson raw_exit "$raw_exit" --argjson raw_reference "$raw_reference" --arg post_source "$post_source" \
    --arg post_ignored "$post_ignored" --arg post_binding "$post_binding" --argjson post_map "$post_map" \
    --arg post_map_sha "$post_map_sha" --arg post_descriptor "$post_descriptor" --argjson output_map "$output_map" '
    def nullable($v): if $v == "" then null else $v end;
    {assertion_digest:$assertion_digest,attribute_to_output_path:$output_map,capability_evidence:$capability,command_identity:$command_identity,descriptor_digest:$descriptor,end_timestamp:$end,gpe_artifact_hash_map:$map,gpe_artifact_hash_map_sha256:$map_sha,invocation_id:$invocation_id,original_invocation_count:1,post_action_descriptor_digest:nullable($post_descriptor),post_action_generated_evidence_binding_sha256:nullable($post_binding),post_action_gpe_artifact_hash_map:$post_map,post_action_gpe_artifact_hash_map_sha256:nullable($post_map_sha),post_action_ignored_inventory_sha256:nullable($post_ignored),post_action_source_snapshot:nullable($post_source),raw_evidence_reference:$raw_reference,raw_exit:$raw_exit,reservation_id:$reservation_id,session:$session,source_snapshot:$source_snapshot,start_timestamp:$start,target_descriptor_id:$target,terminal_failure_class:nullable($terminal_failure),terminal_state:$terminal_state}' )"
  record_sha="$(publish_hashed_json "$terminal_payload" record_sha256 "$repo_root/$terminal" "$reservation/terminal.json")"
  receipt_payload="$(jq -cnS --arg session "$GENERATION" --arg assertion_digest "$assertion_digest" --arg source_snapshot "$(jq -r '.source_snapshot' <<<"$state")" \
    --argjson map "$(jq -c '.gpe_artifact_hash_map' <<<"$state")" --arg map_sha "$map_sha" --arg target "$target" --arg descriptor "$descriptor" \
    --arg terminal "$terminal" --arg record_sha "$record_sha" --arg reservation_id "$reservation_id" --arg terminal_state "$terminal_state" --argjson returned_exit "$returned_exit" '
    {assertion_digest:$assertion_digest,call_kind:"first",current_call_invocation_count:1,descriptor_digest:$descriptor,gpe_artifact_hash_map:$map,gpe_artifact_hash_map_sha256:$map_sha,record_path:$terminal,record_sha256:$record_sha,reservation_id:$reservation_id,returned_exit:$returned_exit,returned_terminal_state:$terminal_state,session:$session,source_snapshot:$source_snapshot,target_descriptor_id:$target}' )"
  receipt_sha="$(publish_hashed_json "$receipt_payload" receipt_sha256 "$repo_root/$first" "$reservation/first-receipt.json")"
  jq -cnS --arg descriptor_digest "$descriptor" --arg first_receipt_path "$first" --arg first_receipt_sha256 "$receipt_sha" \
    --arg record_sha256 "$record_sha" --arg target_descriptor_id "$target" --arg terminal_path "$terminal" --arg terminal_state "$terminal_state" \
    --argjson returned_exit "$returned_exit" '{descriptor_digest:$descriptor_digest,first_receipt_path:$first_receipt_path,first_receipt_sha256:$first_receipt_sha256,record_sha256:$record_sha256,returned_exit:$returned_exit,target_descriptor_id:$target_descriptor_id,terminal_path:$terminal_path,terminal_state:$terminal_state}'
  (( returned_exit == 0 )) || exit "$returned_exit"
}

run_descriptor() {
  local system="$1" requested_map="$2" call_kind="$3" state values paths
  case "$system" in x86_64-linux|aarch64-linux|aarch64-darwin) ;; *) reject 'descriptor system is not allowlisted' ;; esac
  validate_descriptor_contract "$system"
  is_hex "$requested_map" || reject 'descriptor GPE-map digest must be lowercase 64-hex'
  [[ "${TASK13_R25_GPE_MAP_SHA256:-}" == "$requested_map" ]] || reject 'descriptor GPE-map environment receipt differs from argv'
  case "$call_kind" in first|fv07-replay) ;; *) reject 'descriptor call kind must be first or fv07-replay' ;; esac
  state="$(require_current_snapshot "$requested_map")"
  values="$(descriptor_target_values "$system")"
  paths="$(descriptor_paths "$system")"
  if [[ "$call_kind" == fv07-replay ]]; then
    run_descriptor_replay "$system" "$requested_map" "$state" "$values" "$paths"
  else
    run_descriptor_first "$system" "$requested_map" "$state" "$values" "$paths"
  fi
}

(( $# >= 1 )) || usage
readonly subcommand="$1"
shift
case "$subcommand" in
  prepare-snapshot)
    (($# == 1)) || usage
    verify_bootstrap "$1"
    prepare_snapshot
    ;;
  verify-snapshot)
    (($# == 2)) || usage
    verify_bootstrap "$1"
    verify_snapshot "$2"
    ;;
  run-descriptor)
    (($# == 4)) || usage
    verify_bootstrap "$1"
    run_descriptor "$2" "$3" "$4"
    ;;
  *) usage ;;
esac
