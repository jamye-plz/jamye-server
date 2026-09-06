#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

# Internal-only R25 FV01..FV08 dispatcher.  The copied guard validates the
# lowercase public selector; this helper is the sole lowercase -> uppercase
# conversion owner and delegates snapshot verification to the copied final-tree
# helper before and after every step.

readonly GENERATION='task13-r25-g0-f13-trust-root-cross-binding-20260907'
readonly MANIFEST_REL='tests/nix/task-13-assertion-manifest-r25-g0-f13.json'
readonly LOCK_REL='tests/nix/task-13-assertion-manifest-r25-g0-f13.sha256'
readonly RECORD_REL='.agents/results/task-13-s2-r25-g0-f13-manifest-digest-20260907.txt'
readonly SELF_REL='scripts/tasks/task-13/final-verify-r25.sh'
readonly TREE_REL='scripts/tasks/task-13/final-tree-record-r25.sh'
readonly STRICT_REL='scripts/tasks/task-13/strict-entrypoint-r25.sh'
readonly SHAPE_REL='scripts/tasks/task-13/r25-shape.just'
readonly HANDOFF_REL='.agents/results/task-13-r25-final-verify-handoff-20260907.json'

readonly -a FV_ORDER=(FV01 FV02 FV03 FV04 FV05 FV06 FV07 FV08)
readonly -a FV_IDS=(
  fv-01-pf1-fingerprint
  fv-02-operation-event-matrix
  fv-03-consolidated-whole-tree
  fv-04-cross-feature-recovery-privacy-lease-voice
  fv-05-migration-adr-chain
  fv-06-c2-provenance
  fv-07-supported-system-builder-matrix
  fv-08-dependency-license-log-gitleaks
)

repo_root=''
manifest=''
assertion_digest=''
tree_copy=''
tree_sha=''
strict_copy=''
strict_sha=''
gpe_map_sha=''
stage_dir=''

reject() { printf 'error: %s\n' "$1" >&2; exit 2; }
sha256_file() { sha256sum "$1" | awk '{print $1}'; }
sha256_stream() { sha256sum | awk '{print $1}'; }
identity() { stat -c '%d:%i' -- "$1"; }
mode() { stat -c '%a' -- "$1"; }
is_hex() { [[ "$1" =~ ^[0-9a-f]{64}$ ]]; }

usage() {
  printf '%s\n' 'usage: copied final-verify-r25.sh <digest> <fv01|fv02|fv03|fv04|fv05|fv06|fv07|fv08>' >&2
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

# Repository-owned contract inputs are intentionally human-readable JSON, not
# protocol evidence.  They still need an unambiguous object parse, but their
# whitespace/key order is not part of the canonical evidence-file ABI.
require_json_object() {
  local path="$1" label="$2"
  require_regular_nonlink "$path" "$label"
  reject_duplicate_members "$path"
  jq -e 'type == "object"' "$path" >/dev/null 2>&1 || reject "$label root is not an object"
}

manifest_source_sha() {
  local relative="$1"
  jq -er --arg path "$relative" '
    [.command_source_integrity.sources[] | select(.path == $path)]
    | if length == 1 and .[0].type == "regular_non_symlink" and (.[0].sha256 | test("^[0-9a-f]{64}$"))
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
  local digest="$1" self_copy self_sha invocation expected
  is_hex "$digest" || reject 'assertion digest must be lowercase 64-hex'
  [[ "${TASK13_ASSERTION_GENERATION:-}" == "$GENERATION" ]] || reject 'guarded assertion generation receipt is missing or stale'
  [[ "${TASK13_ASSERTION_DIGEST:-}" == "$digest" ]] || reject 'guarded assertion digest receipt differs from argv'
  [[ -z "${TASK13_FLAKE_SOURCE_SNAPSHOT:-}" ]] || reject 'live R25 final-verify helper received an immutable-snapshot receipt'
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
    and .final_tree_record_protocol.final_verify_selector_abi.mapping == [
      {external:"fv01",internal:"FV01"},{external:"fv02",internal:"FV02"},
      {external:"fv03",internal:"FV03"},{external:"fv04",internal:"FV04"},
      {external:"fv05",internal:"FV05"},{external:"fv06",internal:"FV06"},
      {external:"fv07",internal:"FV07"},{external:"fv08",internal:"FV08"}
    ]
    and .final_tree_record_protocol.helper_abi.environment == [
      "TASK13_R25_FINAL_TREE_HELPER_COPY","TASK13_R25_FINAL_TREE_HELPER_SHA256",
      "TASK13_R25_FINAL_VERIFY_HELPER_COPY","TASK13_R25_FINAL_VERIFY_HELPER_SHA256",
      "TASK13_R25_STRICT_ENTRYPOINT_COPY","TASK13_R25_STRICT_ENTRYPOINT_SHA256",
      "TASK13_R25_SELECTED_JUSTFILE_COPY","TASK13_R25_SELECTED_JUSTFILE_SHA256",
      "TASK13_R25_SELECTED_JUSTFILE_IDENTITY","TASK13_R25_GPE_MAP_SHA256"
    ]
    and .final_tree_record_protocol.helper_abi.generic_recipe_receipts == [
      "TASK13_ASSERTION_GENERATION","TASK13_ASSERTION_DIGEST","TASK13_FLAKE_SOURCE_SNAPSHOT",
      "TASK13_BOUND_JUSTFILE","TASK13_BOUND_JUSTFILE_IDENTITY","TASK13_BOUND_JUSTFILE_SHA256"
    ]
    and .operator_surfaces.final_verify.handoff_fields == [
      "assertion_digest","source_snapshot","gpe_artifact_hash_map","gpe_artifact_hash_map_sha256",
      "repository_manifest_sha256","ignored_input_inventory_sha256","input_coverage_oracle_sha256",
      "source_snapshot_hash_artifact_sha256","generated_evidence_binding_sha256",
      "fv_input_sets_canonical_sha256","fv_output_sets_canonical_sha256",
      "fv01_fv08_ordered_step_input_digests","fv01_fv08_pre_post_input_set_equality",
      "fv01_fv08_gpe_map_pre_post_equality","fv01_fv08_output_hashes_and_verdicts",
      "fv01_fv08_raw_result_sha256","fv01_expected_selector_sha256",
      "fv01_observed_legacy_snapshot_sha256","fv01_expected_comparison_verdict",
      "linux_target_descriptor_id","linux_descriptor_digest","linux_terminal_path",
      "linux_record_sha256","linux_replay_receipt_sha256",
      "aarch64_linux_target_descriptor_id","aarch64_linux_descriptor_digest",
      "aarch64_linux_terminal_path","aarch64_linux_record_sha256",
      "aarch64_linux_replay_receipt_sha256","darwin_target_descriptor_id",
      "darwin_descriptor_digest","darwin_terminal_path","darwin_record_sha256",
      "darwin_replay_receipt_sha256","per_step_exit_evidence_references","no_mutation_declarations"
    ]
  ' "$manifest" >/dev/null || reject 'materialized R25 final-verify protocol is malformed'

  self_copy="${TASK13_R25_FINAL_VERIFY_HELPER_COPY:-}"
  self_sha="${TASK13_R25_FINAL_VERIFY_HELPER_SHA256:-}"
  invocation="$(physical_file "${BASH_SOURCE[0]}")" || reject 'helper invocation path cannot be resolved'
  [[ "$invocation" == "$self_copy" ]] || reject 'final-verify helper was not invoked from the guarded external copy'
  require_external_copy "$self_copy" "$self_sha" 'final-verify helper'
  expected="$(manifest_source_sha "$SELF_REL")" || reject 'manifest final-verify source row is invalid'
  [[ "$self_sha" == "$expected" ]] || reject 'final-verify helper receipt is not manifest-bound'

  tree_copy="${TASK13_R25_FINAL_TREE_HELPER_COPY:-}"
  tree_sha="${TASK13_R25_FINAL_TREE_HELPER_SHA256:-}"
  require_external_copy "$tree_copy" "$tree_sha" 'final-tree helper'
  expected="$(manifest_source_sha "$TREE_REL")" || reject 'manifest final-tree source row is invalid'
  [[ "$tree_sha" == "$expected" ]] || reject 'final-tree helper receipt is not manifest-bound'

  strict_copy="${TASK13_R25_STRICT_ENTRYPOINT_COPY:-}"
  strict_sha="${TASK13_R25_STRICT_ENTRYPOINT_SHA256:-}"
  require_external_copy "$strict_copy" "$strict_sha" 'strict entrypoint'
  expected="$(manifest_source_sha "$STRICT_REL")" || reject 'manifest strict-entrypoint source row is invalid'
  [[ "$strict_sha" == "$expected" ]] || reject 'strict-entrypoint receipt is not manifest-bound'
  verify_selected_justfile_receipts
  gpe_map_sha="${TASK13_R25_GPE_MAP_SHA256:-}"
  is_hex "$gpe_map_sha" || reject 'verified GPE-map digest receipt is missing'
}

output_path() {
  case "$1" in
    fv01-observed-legacy-snapshot) printf '%s' '.agents/results/task-13-fv01-observed-legacy-snapshot-20260902-090159.nul' ;;
    fv01-raw-result) printf '%s' '.agents/results/task-13-fv01-pf1-raw-result-20260902-090159.txt' ;;
    fv02-raw-result) printf '%s' '.agents/results/task-13-fv02-matrix-raw-result-20260902-090159.txt' ;;
    fv03-whole-tree-raw-result) printf '%s' '.agents/results/task-13-fv03-consolidated-whole-tree-20260902-090159.txt' ;;
    fv04-raw-result) printf '%s' '.agents/results/task-13-fv04-recovery-raw-result-20260902-090159.txt' ;;
    fv05-migration-adr-audit-evidence) printf '%s' '.agents/results/task-13-fv05-migration-adr-audit-20260902-090159.json' ;;
    fv06-c2-provenance-evidence) printf '%s' '.agents/results/task-13-fv06-c2-provenance-20260902-090159.json' ;;
    fv07-raw-result) printf '%s' '.agents/results/task-13-fv07-builder-matrix-raw-result-20260902-090159.txt' ;;
    linux-replay-receipt) printf '%s' '.agents/results/task-13-r25-x86_64-linux-fv07-replay-receipt-20260907.json' ;;
    aarch64-linux-replay-receipt) printf '%s' '.agents/results/task-13-r25-aarch64-linux-fv07-replay-receipt-20260907.json' ;;
    darwin-replay-receipt) printf '%s' '.agents/results/task-13-r25-aarch64-darwin-fv07-replay-receipt-20260907.json' ;;
    fv08-dependency-evidence) printf '%s' '.agents/results/task-13-fv08-dependency-20260902-090159.txt' ;;
    fv08-license-evidence) printf '%s' '.agents/results/task-13-fv08-license-20260902-090159.txt' ;;
    fv08-log-gitleaks-evidence) printf '%s' '.agents/results/task-13-fv08-sensitive-log-gitleaks-20260902-090159.txt' ;;
    protocol-linux-terminal-record) printf '%s' '.agents/results/task-13-r25-x86_64-linux-terminal-20260907.json' ;;
    protocol-linux-first-receipt) printf '%s' '.agents/results/task-13-r25-x86_64-linux-first-receipt-20260907.json' ;;
    protocol-aarch64-linux-terminal-record) printf '%s' '.agents/results/task-13-r25-aarch64-linux-terminal-20260907.json' ;;
    protocol-aarch64-linux-first-receipt) printf '%s' '.agents/results/task-13-r25-aarch64-linux-first-receipt-20260907.json' ;;
    protocol-darwin-terminal-record) printf '%s' '.agents/results/task-13-r25-aarch64-darwin-terminal-20260907.json' ;;
    protocol-darwin-first-receipt) printf '%s' '.agents/results/task-13-r25-aarch64-darwin-first-receipt-20260907.json' ;;
    protocol-final-handoff) printf '%s' "$HANDOFF_REL" ;;
    *) reject "unknown R25 protocol output ID: $1" ;;
  esac
}

verify_manifest_output_paths() {
  local id expected actual
  while IFS= read -r id; do
    expected="$(output_path "$id")"
    actual="$(jq -er --arg id "$id" '.final_tree_snapshot.catalog_protocol_output_entries[] | select(.input_id == $id) | .path_or_external_selector' "$manifest")" || reject "manifest protocol-output row is missing for $id"
    [[ "$actual" == "$expected" ]] || reject "manifest protocol-output path differs for $id"
  done < <(jq -r '.final_tree_snapshot.catalog_protocol_output_entries[].input_id' "$manifest")
}

payload_hash() {
  local path="$1" field="$2"
  jq -cS "del(.$field)" "$path" | sha256_stream
}

validate_json_hash() {
  local path="$1" field="$2" label="$3" displayed
  require_canonical_object "$path" "$label"
  displayed="$(jq -er --arg field "$field" '.[$field] | select(type == "string" and test("^[0-9a-f]{64}$"))' "$path")" || reject "$label displayed hash is malformed"
  [[ "$(payload_hash "$path" "$field")" == "$displayed" ]] || reject "$label payload hash differs"
}

validate_output() {
  local id="$1" path relative
  relative="$(output_path "$id")"; path="$repo_root/$relative"
  require_regular_nonlink "$path" "FV output $id"
  case "$id" in
    *-replay-receipt|protocol-*-first-receipt)
      validate_json_hash "$path" receipt_sha256 "$id"
      jq -e '.returned_terminal_state == "build_succeeded" and .returned_exit == 0' "$path" >/dev/null || reject "$id is not a successful receipt" ;;
    protocol-*-terminal-record)
      validate_json_hash "$path" record_sha256 "$id"
      jq -e '.terminal_state == "build_succeeded" and .attribute_to_output_path != null' "$path" >/dev/null || reject "$id is not a successful terminal" ;;
    fv05-migration-adr-audit-evidence|fv06-c2-provenance-evidence)
      require_canonical_object "$path" "$id"
      jq -e '.verdict == "PASS"' "$path" >/dev/null || reject "$id verdict is not PASS" ;;
    fv01-observed-legacy-snapshot)
      [[ -s "$path" ]] || reject 'FV01 observed legacy snapshot is empty' ;;
    *)
      [[ -s "$path" ]] || reject "$id output is empty"
      [[ "$(tail -n 1 "$path")" == 'VERDICT: PASS' ]] || reject "$id final verdict is not PASS" ;;
  esac
}

verify_output_sequence() {
  local current_index="$1" index fv id path
  [[ ! -e "$repo_root/$HANDOFF_REL" && ! -L "$repo_root/$HANDOFF_REL" ]] || reject 'R25 final handoff already exists before FV dispatch'
  for index in 0 1 2 3 4 5 6 7; do
    fv="${FV_ORDER[$index]}"
    while IFS= read -r id; do
      path="$repo_root/$(output_path "$id")"
      if (( index < current_index )); then
        validate_output "$id"
      else
        [[ ! -e "$path" && ! -L "$path" ]] || reject "$id must be absent before ${FV_ORDER[$current_index]} dispatch"
      fi
    done < <(jq -r --arg fv "$fv" '.final_tree_record_protocol.fv_output_sets[$fv][]' "$manifest")
  done
}

verify_tree_snapshot() {
  local fv="$1"
  TASK13_R25_GPE_MAP_SHA256="$gpe_map_sha" bash "$tree_copy" verify-snapshot "$assertion_digest" "$fv"
}

exclusive_publish() {
  local staged="$1" relative="$2" label="$3" destination="$repo_root/$relative"
  [[ -f "$staged" && ! -L "$staged" ]] || reject "$label staged output is missing"
  [[ ! -e "$destination" && ! -L "$destination" ]] || reject "$label destination already exists"
  chmod 0400 -- "$staged"
  ln -- "$staged" "$destination" || reject "$label exclusive publication failed"
  cmp -s "$staged" "$destination" || reject "$label bytes differ after publication"
  rm -f -- "$staged"
}

append_nested_pair() {
  local raw_file="$1" justfile="$2" recipe="$3"; shift 3
  local stdout_file="$stage_dir/nested-$RANDOM.stdout" stderr_file="$stage_dir/nested-$RANDOM.stderr" status
  local -a argv=(bash "$strict_copy" "$assertion_digest" pair "$justfile" "$recipe")
  (($# == 0)) || argv+=(-- "$@")
  printf 'COMMAND: %s::%s' "$justfile" "$recipe" >>"$raw_file"
  printf ' %q' "$@" >>"$raw_file"
  printf '\n' >>"$raw_file"
  set +e
  TASK13_R25_ENTRYPOINT_SHA256="$strict_sha" "${argv[@]}" >"$stdout_file" 2>"$stderr_file"
  status=$?
  set -e
  printf 'EXIT: %s\nSTDOUT_BEGIN\n' "$status" >>"$raw_file"
  cat -- "$stdout_file" >>"$raw_file"
  printf '%s\n' 'STDOUT_END' >>"$raw_file"
  printf 'STDERR_SHA256: %s\n' "$(sha256_file "$stderr_file")" >>"$raw_file"
  if [[ -s "$stderr_file" ]]; then
    printf '%s\n' 'STDERR_BEGIN' >>"$raw_file"
    cat -- "$stderr_file" >>"$raw_file"
    printf '%s\n' 'STDERR_END' >>"$raw_file"
  fi
  return "$status"
}

run_pairs() {
  local raw_file="$1"; shift
  local specification justfile recipe args status overall=0
  for specification in "$@"; do
    IFS='|' read -r justfile recipe args <<<"$specification"
    set +e
    if [[ -n "$args" ]]; then
      # Approved argument-bearing pairs use comma-free whitespace-safe tokens.
      read -r -a split_args <<<"$args"
      append_nested_pair "$raw_file" "$justfile" "$recipe" "${split_args[@]}"
    else
      append_nested_pair "$raw_file" "$justfile" "$recipe"
    fi
    status=$?
    set -e
    (( status == 0 )) || overall=1
  done
  return "$overall"
}

finish_raw() {
  local path="$1" verdict="$2"
  printf 'VERDICT: %s\n' "$verdict" >>"$path"
}

document_selector_sha() {
  jq -cS '[.final_tree_snapshot.catalog_fixed_entries[] | select(.input_id == "task2-pf1-baseline-sha-fingerprint" or .input_id == "task2-pf1-prompt-span-catalog") | {input_id,literal_external_selector}]' "$manifest" | sha256_stream
}

build_legacy_snapshot() {
  local destination="$1" pf legacy_root list_file unsorted_list relative absolute kind file_mode content_sha spec doc span path_sha entry value
  local head status_file status_sha status_count row_count=0 observed_tsv expected_tsv baseline
  pf="$(jq -c '.operator_surfaces.final_verify.authoritative_inputs.pf1' "$manifest")"
  legacy_root="$(jq -r '.legacy_root' <<<"$pf")"
  [[ "$legacy_root" == /* && -d "$legacy_root" && ! -L "$legacy_root" ]] || reject 'FV01 frozen legacy root is missing or linked'
  list_file="$stage_dir/legacy-paths.nul"
  unsorted_list="$stage_dir/legacy-paths-unsorted.nul"
  git -C "$legacy_root" ls-files --cached --others --exclude-standard -z -- backend/app backend/alembic/versions backend/tests \
    | while IFS= read -r -d '' relative; do
        case "$relative" in *.py) case "$relative" in *__pycache__*|*.pytest_cache*|*.pyc|*.pyo) continue;; esac;; *) continue;; esac
        printf '%s\0' "$relative"
      done >"$unsorted_list"
  while IFS= read -r spec; do
    if [[ "$spec" =~ ^(.+)[[:space:]]lines[[:space:]][0-9]+-[0-9]+$ ]]; then doc="${BASH_REMATCH[1]}"; else doc="$spec"; fi
    printf '%s\0' "$doc" >>"$unsorted_list"
  done < <(jq -r '.documents[]' <<<"$pf")
  LC_ALL=C sort -zu "$unsorted_list" >"$list_file"
  observed_tsv="$stage_dir/legacy-observed.tsv"; expected_tsv="$stage_dir/legacy-expected.tsv"
  : >"$observed_tsv"
  : >"$destination"
  while IFS= read -r -d '' relative; do
    absolute="$legacy_root/$relative"
    [[ -f "$absolute" && ! -L "$absolute" ]] || reject "FV01 legacy regular file is missing or linked: $relative"
    case "$relative" in
      backend/alembic/versions/*.py) kind=migration ;;
      backend/app/*.py|backend/app/**/*.py) kind=app_source ;;
      backend/tests/*.py|backend/tests/**/*.py) kind=legacy_test ;;
      docs/greenfield/jamye-app-initial-prompt.md) kind='prompt(requirements=lines 1-319; full SHA auxiliary)' ;;
      docs/greenfield/jamye-server-initial-prompt.md) kind='prompt(requirements=lines 8-304; full SHA auxiliary)' ;;
      docs/*) kind=authoritative_doc ;;
      *) reject "FV01 legacy path has no frozen kind: $relative" ;;
    esac
    file_mode="$(mode "$absolute")"; content_sha="$(sha256_file "$absolute")"
    printf 'legacy-path\0%s\0%s\0%s\0%s\0' "$relative" "$kind" "$file_mode" "$content_sha" >>"$destination"
    printf '%s\t%s\t%s\n' "$relative" "$kind" "$content_sha" >>"$observed_tsv"
    row_count=$((row_count + 1))
  done <"$list_file"
  while IFS= read -r spec; do
    [[ -n "$spec" ]] || continue
    if [[ "$spec" =~ ^(.+)[[:space:]]lines[[:space:]]([0-9]+)-([0-9]+)$ ]]; then
      doc="${BASH_REMATCH[1]}"; span="${BASH_REMATCH[2]}-${BASH_REMATCH[3]}"
      require_regular_nonlink "$legacy_root/$doc" "FV01 document selector $doc"
      path_sha="$(sed -n "${BASH_REMATCH[2]},${BASH_REMATCH[3]}p" "$legacy_root/$doc" | sha256_stream)"
    else
      doc="$spec"; span=all
      require_regular_nonlink "$legacy_root/$doc" "FV01 document selector $doc"
      path_sha="$(sha256_file "$legacy_root/$doc")"
    fi
    printf 'document-selector\0%s\0%s\0%s\0' "$doc" "$span" "$path_sha" >>"$destination"
  done < <(jq -r '.documents[]' <<<"$pf")
  while IFS= read -r value; do printf 'recursive-glob\0%s\0' "$value" >>"$destination"; done < <(jq -r '.recursive_globs[]' <<<"$pf")
  while IFS= read -r value; do printf 'exclusion\0%s\0' "$value" >>"$destination"; done < <(jq -r '.exclusions[]' <<<"$pf")
  head="$(git -C "$legacy_root" rev-parse --verify HEAD)"
  status_file="$stage_dir/legacy-status.nul"
  git -C "$legacy_root" status --porcelain=v1 -z | LC_ALL=C sort -z >"$status_file"
  status_sha="$(sha256_file "$status_file")"
  status_count="$(tr -cd '\0' <"$status_file" | wc -c | tr -d ' ')"
  while IFS= read -r -d '' entry; do printf 'status-entry\0%s\0' "$entry" >>"$destination"; done <"$status_file"
  printf 'legacy-head\0%s\0regular-file-rows\0%s\0status-fingerprint\0%s\0status-entries\0%s\0' "$head" "$row_count" "$status_sha" "$status_count" >>"$destination"
  baseline="$repo_root/$(jq -r '.baseline_artifact' <<<"$pf")"
  awk -F '\t' -v root="$legacy_root/" '
    $0 == "~~~tsv" {inside=1; next}
    inside && $0 == "~~~" {exit}
    inside && $1 != "sha256" {
      path=$3
      if (index(path,root) == 1) path=substr(path,length(root)+1)
      print path "\t" $2 "\t" $1
    }
  ' "$baseline" | LC_ALL=C sort >"$expected_tsv"
  LC_ALL=C sort "$observed_tsv" >"$stage_dir/legacy-observed-sorted.tsv"
  cmp -s "$stage_dir/legacy-observed-sorted.tsv" "$expected_tsv" || reject 'FV01 legacy path/kind/SHA set differs bidirectionally from the frozen PF1 manifest'
  [[ "$head" == "$(jq -r '.legacy_head' <<<"$pf")" \
    && "$row_count" == "$(jq -r '.regular_file_rows' <<<"$pf")" \
    && "$status_sha" == "$(jq -r '.status_fingerprint' <<<"$pf")" \
    && "$status_count" == "$(jq -r '.status_entries' <<<"$pf")" \
    && "$(sha256_file "$baseline")" == "$(jq -r '.baseline_sha256' <<<"$pf")" ]] \
    || reject 'FV01 observed legacy selectors differ from the frozen PF1 authority'
}

run_fv01() {
  local observed="$stage_dir/fv01-observed.nul" observed_again="$stage_dir/fv01-observed-again.nul" raw="$stage_dir/fv01-raw.txt"
  build_legacy_snapshot "$observed"
  build_legacy_snapshot "$observed_again"
  cmp -s "$observed" "$observed_again" || reject 'FV01 legacy observation drifted during the audit'
  printf 'FV: FV01\nEXPECTED_SELECTOR_SHA256: %s\nOBSERVED_LEGACY_SNAPSHOT_SHA256: %s\n' "$(document_selector_sha)" "$(sha256_file "$observed")" >"$raw"
  finish_raw "$raw" PASS
}

run_fv02() {
  local raw="$stage_dir/fv02-raw.txt" baseline mapping contract operations events reference reference_path verdict=PASS
  baseline="$(jq -r '.operator_surfaces.final_verify.authoritative_inputs.task2_matrix.baseline_artifact' "$manifest")"
  mapping="$repo_root/contracts/fixtures/selected-surface-mapping.json"; contract="$repo_root/contracts/manifest.json"
  require_regular_nonlink "$repo_root/$baseline" 'FV02 baseline'
  [[ "$(sha256_file "$repo_root/$baseline")" == "$(jq -r '.operator_surfaces.final_verify.authoritative_inputs.task2_matrix.baseline_sha256' "$manifest")" ]] || verdict=FAIL
  while IFS= read -r section; do LC_ALL=C rg -F -q -- "$section" "$repo_root/$baseline" || verdict=FAIL; done < <(jq -r '.operator_surfaces.final_verify.authoritative_inputs.task2_matrix.sections[]' "$manifest")
  require_json_object "$mapping" 'selected surface mapping'
  require_json_object "$contract" 'contract manifest'
  operations="$(jq '.rest_operations|length' "$mapping")"; events="$(jq '.realtime_events|length' "$mapping")"
  [[ "$operations" == 42 && "$events" == 2 ]] || verdict=FAIL
  jq -e '
    (keys | sort) == ["realtime_events","rest_operations"]
    and (.rest_operations | length == 42
      and ([.[].operation_id] | length == (unique | length))
      and all(.[];
        (keys | sort) == ["feature_behavior_test","fixture","handler","handler_route_probe","method","operation_id","path"]
        and all(.feature_behavior_test,.fixture,.handler,.handler_route_probe,.method,.operation_id,.path;
          type == "string" and length > 0)))
    and (.realtime_events | length == 2
      and ([.[].event_type] | length == (unique | length))
      and all(.[];
        (keys | sort) == ["event_type","feature_behavior_test","fixture","handler","handler_route_probe","schema","version"]
        and all(.event_type,.feature_behavior_test,.fixture,.handler,.handler_route_probe,.schema;
          type == "string" and length > 0)
        and (.version | type == "number" and . > 0 and floor == .)))
  ' "$mapping" >/dev/null || verdict=FAIL
  while IFS= read -r reference; do
    reference_path="${reference%%::*}"
    [[ "$reference_path" != /* && "$reference_path" != *'..'* ]] || { verdict=FAIL; continue; }
    [[ -f "$repo_root/$reference_path" && ! -L "$repo_root/$reference_path" ]] || verdict=FAIL
  done < <(jq -r '[.rest_operations[],.realtime_events[]][]
    | [.handler,.feature_behavior_test,.handler_route_probe,.fixture,(.schema // empty)][]' "$mapping")
  jq -e --argjson mapping "$(jq -c '.' "$mapping")" '(.operation_ids == ($mapping.rest_operations|map(.operation_id))) and (.realtime_discriminants == ($mapping.realtime_events|map(.event_type)))' "$contract" >/dev/null || verdict=FAIL
  printf 'FV: FV02\nREST_OPERATIONS: %s\nREALTIME_EVENTS: %s\n' "$operations" "$events" >"$raw"
  finish_raw "$raw" "$verdict"
  [[ "$verdict" == PASS ]]
}

contract_manifest_checksum() {
  local contract="$1" core="$stage_dir/contract-manifest-core.json" path absolute byte_length
  jq -e '
    (.artifacts | type == "array" and length > 0 and . == (sort) and length == (unique | length))
    and (.artifacts | index("manifest.json") != null)
    and (.sha256 | type == "string" and test("^[0-9a-f]{64}$"))
  ' "$contract" >/dev/null || return 1
  jq -cjS 'del(.sha256)' "$contract" >"$core" || return 1
  while IFS= read -r path; do
    [[ -n "$path" && "$path" != /* && "$path" != *'..'* ]] || return 1
    if [[ "$path" == manifest.json ]]; then absolute="$core"; else absolute="$repo_root/contracts/$path"; fi
    [[ -f "$absolute" && ! -L "$absolute" ]] || return 1
  done < <(jq -r '.artifacts[]' "$contract")
  {
    while IFS= read -r path; do
      if [[ "$path" == manifest.json ]]; then absolute="$core"; else absolute="$repo_root/contracts/$path"; fi
      byte_length="$(wc -c <"$absolute" | tr -d ' ')"
      printf '%s\0%s\0' "$path" "$byte_length"
      cat -- "$absolute"
    done < <(jq -r '.artifacts[]' "$contract")
  } | sha256_stream
}

run_fv03() {
  local raw="$stage_dir/fv03-raw.txt" verdict=PASS
  : >"$raw"
  run_pairs "$raw" \
    'scripts/tasks/task-1/mod.just|platform-check|' \
    'scripts/tasks/task-12/mod.just|composition-green|' \
    'scripts/tasks/task-12/mod.just|migration-chain-green|' \
    'scripts/tasks/task-3b/mod.just|check|' \
    'scripts/tasks/task-13/r25-shape.just|coverage-r25-exec|' || verdict=FAIL
  finish_raw "$raw" "$verdict"
  [[ "$verdict" == PASS ]]
}

run_fv04() {
  local raw="$stage_dir/fv04-raw.txt" verdict=PASS
  : >"$raw"
  run_pairs "$raw" \
    'scripts/tasks/task-4a/mod.just|postgres-recovery|' \
    'scripts/tasks/task-4b/mod.just|redis-recovery|' \
    'scripts/tasks/task-4b/mod.just|redis-publish-config-green|' \
    'scripts/tasks/task-8/mod.just|resilience-green|' \
    'scripts/tasks/task-8/mod.just|contract-green|' \
    'scripts/tasks/task-9/mod.just|delivery-lifecycle-green|' \
    'scripts/tasks/task-9/mod.just|privacy-mutations-green|' \
    'scripts/tasks/task-11/mod.just|push-barriers-green|' || verdict=FAIL
  finish_raw "$raw" "$verdict"
  [[ "$verdict" == PASS ]]
}

migration_owner() {
  case "$1" in
    0001) printf '%s' 'docs/commands/task-3a/core-schema.md' ;;
    0002) printf '%s' 'docs/commands/task-5/auth.md' ;;
    0003) printf '%s' 'docs/commands/task-6/groups.md' ;;
    0004) printf '%s' 'docs/commands/task-6b/chatrooms.md' ;;
    0005) printf '%s' 'docs/commands/task-7/topics.md' ;;
    0006) printf '%s' 'docs/commands/task-8/media.md' ;;
    0007) printf '%s' 'docs/commands/task-9/notifications.md' ;;
    0008) printf '%s' 'docs/commands/task-11/account-deletion.md' ;;
    *) return 1 ;;
  esac
}

make_fv05_audit() {
  local destination="$1" verdict="$2" source="$3" rows="$stage_dir/migration-rows.jsonl" id path owner adr recovery all_valid=true topic_line fk_line
  : >"$rows"
  for id in 0001 0002 0003 0004 0005 0006 0007 0008; do
    path="$(printf '%s\n' "$repo_root/migrations/${id}_"*.sql)"
    [[ -f "$path" && ! -L "$path" && "$path" != *$'\n'* ]] || all_valid=false
    owner="$(migration_owner "$id")" || all_valid=false
    [[ -f "$repo_root/$owner" && ! -L "$repo_root/$owner" ]] || all_valid=false
    if LC_ALL=C rg -q '^-- migration:' "$path" \
      && LC_ALL=C rg -q '^-- prerequisite:' "$path" \
      && LC_ALL=C rg -q '^-- reversibility:' "$path" \
      && LC_ALL=C rg -q '^-- rationale:' "$path"; then adr=true; else adr=false; all_valid=false; fi
    if LC_ALL=C rg -q '^-- recovery: docs/adr/0003-forward-only-sqlx-migrations\.md$' "$path"; then recovery=true; else recovery=false; all_valid=false; fi
    jq -cnS --arg id "$id" --arg path "${path#$repo_root/}" --arg owner "$owner" --arg sha "$(sha256_file "$path")" \
      --argjson adr "$adr" --argjson recovery "$recovery" '{adr_metadata_present:$adr,id:$id,owner_evidence:$owner,path:$path,recovery_reference_present:$recovery,sha256:$sha}' >>"$rows"
  done
  topic_line="$(LC_ALL=C rg -n -m1 '^CREATE TABLE topics' "$repo_root/migrations/0005_topics.sql" | cut -d: -f1)"
  fk_line="$(LC_ALL=C rg -n -m1 'ADD (COLUMN )?topic_id|FOREIGN KEY.*topic' "$repo_root/migrations/0005_topics.sql" | cut -d: -f1)"
  [[ "$topic_line" =~ ^[0-9]+$ && "$fk_line" =~ ^[0-9]+$ && "$topic_line" -lt "$fk_line" ]] || all_valid=false
  [[ "$all_valid" == true ]] || verdict=FAIL
  jq -cS -s --arg digest "$assertion_digest" --arg generation "$GENERATION" --arg source "$source" --arg verdict "$verdict" --argjson topic "$([[ "$all_valid" == true ]] && printf true || printf false)" \
    '{assertion_digest:$digest,canonical_chain:["0001","0002","0003","0004","0005","0006","0007","0008"],generation_id:$generation,migration_rows:.,production_data_touched:false,source_snapshot:$source,topic_fk_order_valid:$topic,verdict:$verdict}' "$rows" >"$destination"
}

run_fv05() {
  local source="$1" audit="$stage_dir/fv05.json" raw="$stage_dir/fv05-nested.txt" verdict=PASS
  : >"$raw"
  # The helper-local migration/ADR audit is the first frozen internal action;
  # the strict migration-chain pair follows it exactly as catalogued.
  make_fv05_audit "$audit" "$verdict" "$source"
  [[ "$(jq -r '.verdict' "$audit")" == PASS ]] || verdict=FAIL
  append_nested_pair "$raw" scripts/tasks/task-12/mod.just migration-chain-green || verdict=FAIL
  if [[ "$verdict" != PASS ]]; then
    jq -cS '.verdict = "FAIL"' "$audit" >"$stage_dir/rewrite-fv05.json"
    mv -- "$stage_dir/rewrite-fv05.json" "$audit"
  fi
  cat -- "$raw"
  [[ "$verdict" == PASS ]]
}

make_fv06_audit() {
  local destination="$1" verdict="$2" source="$3" fixture="$repo_root/contracts/fixtures/manifest-provenance.json"
  local contract="$repo_root/contracts/manifest.json" provenance="$repo_root/src/contract_generation/provenance.json" frozen=true checksum=true expected_checksum actual_checksum
  require_json_object "$fixture" 'manifest provenance fixture'
  require_json_object "$contract" 'generated contract manifest'
  require_json_object "$provenance" 'contract provenance source'
  jq -e '
    .pre_publication.git_head_inference == false
    and .pre_publication.input == {contract_version:"1",server_commit:"dirty",server_tag:null,server_version:"0.1.0"}
    and .pre_publication.clean_checkout_result == "still_uses_the_explicit_dirty_input"
    and .pre_publication.dirty_workspace_result == "uses_the_explicit_input"
    and .future_separately_authorized_publication.current_plan_executes_transition == false
    and .future_separately_authorized_publication.artifact_commit_points_to_itself == false
  ' "$fixture" >/dev/null || frozen=false
  cmp -s <(jq -cS '.pre_publication.input' "$fixture") <(jq -cS '.' "$provenance") || frozen=false
  jq -e '.server_commit == "dirty" and .server_tag == null and (.sha256|test("^[0-9a-f]{64}$")) and (.checksum_algorithm == "sha256 over lexicographic path,NUL,decimal-length,NUL,bytes entries; manifest.json uses recursively key-sorted compact JSON without sha256; v1")' "$contract" >/dev/null || checksum=false
  if [[ "$checksum" == true ]]; then
    actual_checksum="$(jq -er '.sha256' "$contract")"
    expected_checksum="$(contract_manifest_checksum "$contract")" || checksum=false
    [[ "$checksum" != true || "$actual_checksum" == "$expected_checksum" ]] || checksum=false
  fi
  [[ "$frozen" == true && "$checksum" == true ]] || verdict=FAIL
  jq -cnS --arg digest "$assertion_digest" --arg generation "$GENERATION" --arg source "$source" --arg verdict "$verdict" \
    --argjson frozen "$frozen" --argjson checksum "$checksum" '
    {ambient_git_used:false,assertion_digest:$digest,checksum_self_excluding:$checksum,fixture_results:{clean:$frozen,dirty:$frozen,future_transition:$frozen},frozen_source_equal:$frozen,generation_id:$generation,server_commit:"dirty",server_owned_app_contract_lock:false,server_tag:null,source_snapshot:$source,verdict:$verdict}' >"$destination"
}

run_fv06() {
  local source="$1" audit="$stage_dir/fv06.json" raw="$stage_dir/fv06-nested.txt" verdict=PASS
  : >"$raw"
  run_pairs "$raw" 'scripts/tasks/task-3b/mod.just|check|' 'scripts/tasks/task-12/mod.just|contract-green|' || verdict=FAIL
  make_fv06_audit "$audit" "$verdict" "$source"
  verdict="$(jq -r '.verdict' "$audit")"
  cat -- "$raw"
  [[ "$verdict" == PASS ]]
}

run_fv07() {
  local raw="$stage_dir/fv07-raw.txt" verdict=PASS
  : >"$raw"
  run_pairs "$raw" \
    "scripts/tasks/task-13/r25-shape.just|linux-builder-r25-exec|$gpe_map_sha fv07-replay" \
    "scripts/tasks/task-13/r25-shape.just|aarch64-linux-builder-r25-exec|$gpe_map_sha fv07-replay" \
    "scripts/tasks/task-13/r25-shape.just|cross-system-verify-r25-exec|$gpe_map_sha fv07-replay" || verdict=FAIL
  finish_raw "$raw" "$verdict"
  [[ "$verdict" == PASS ]]
}

run_fv08() {
  local dependency="$stage_dir/fv08-dependency.txt" license="$stage_dir/fv08-license.txt" sensitive="$stage_dir/fv08-sensitive.txt"
  local dep_raw="$stage_dir/fv08-dependency-raw.txt" secret_raw="$stage_dir/fv08-secret-raw.txt" verdict=PASS dependency_sensitive=false final_sensitive=false
  local sensitive_ere='(authorization:[[:space:]]*bearer[[:space:]]+[A-Za-z0-9._-]+|x-amz-(signature|credential)=[^[:space:]]+|expo(push)?token=[^[:space:]]+|presigned[_ -]?url=https?://|oauth[_ -]?code=[^[:space:]]+|ticket(_secret)?=[^[:space:]]+|message[_ -]?preview=[^[:space:]]+|(aws_secret_access_key|minio_root_password|s3_secret_key)=[^[:space:]]+)'
  : >"$dep_raw"; : >"$secret_raw"
  append_nested_pair "$dep_raw" scripts/tasks/task-1/mod.just dependency-check || verdict=FAIL
  # A failing scanner must never turn its finding into a persistent secret
  # artifact.  Preserve raw bytes only when the closed sensitive-value audit
  # is clean; otherwise retain only their digest and a typed redaction marker.
  if LC_ALL=C rg -i -q "$sensitive_ere" "$dep_raw"; then
    dependency_sensitive=true; verdict=FAIL
    printf 'REDACTED_DEPENDENCY_OUTPUT_SHA256: %s\nSENSITIVE_DEPENDENCY_MATCH_COUNT: 1\n' "$(sha256_file "$dep_raw")" >"$dependency"
  else
    cp -- "$dep_raw" "$dependency"
  fi
  finish_raw "$dependency" "$verdict"
  printf 'DEPENDENCY_EVIDENCE_SHA256: %s\nLICENSE_POLICY_OWNER: scripts/tasks/task-1/mod.just::dependency-check\n' "$(sha256_file "$dep_raw")" >"$license"
  finish_raw "$license" "$verdict"
  # The helper-local sensitive-log audit is deliberately between the
  # dependency pair and the final secret-scan pair in the frozen order.
  printf 'DEPENDENCY_OUTPUT_SHA256: %s\nSENSITIVE_DEPENDENCY_MATCH_COUNT: %s\n' \
    "$(sha256_file "$dep_raw")" "$([[ "$dependency_sensitive" == false ]] && printf 0 || printf 1)" >"$sensitive"
  append_nested_pair "$secret_raw" scripts/tasks/task-1/mod.just secret-scan || verdict=FAIL
  if LC_ALL=C rg -i -q "$sensitive_ere" "$secret_raw"; then final_sensitive=true; verdict=FAIL; fi
  printf 'SECRET_SCAN_OUTPUT_SHA256: %s\nSENSITIVE_FINAL_MATCH_COUNT: %s\n' \
    "$(sha256_file "$secret_raw")" "$([[ "$final_sensitive" == false ]] && printf 0 || printf 1)" >>"$sensitive"
  if [[ "$final_sensitive" == false ]]; then
    cat -- "$secret_raw" >>"$sensitive"
  else
    printf '%s\n' 'SECRET_SCAN_OUTPUT: REDACTED_SENSITIVE_FINDING' >>"$sensitive"
  fi
  finish_raw "$sensitive" "$verdict"
  # The late sensitive audit participates in all three verdicts.
  if [[ "$verdict" != PASS ]]; then
    head -n -1 "$dependency" >"$stage_dir/rewrite-dependency"; printf '%s\n' 'VERDICT: FAIL' >>"$stage_dir/rewrite-dependency"; mv -- "$stage_dir/rewrite-dependency" "$dependency"
    head -n -1 "$license" >"$stage_dir/rewrite-license"; printf '%s\n' 'VERDICT: FAIL' >>"$stage_dir/rewrite-license"; mv -- "$stage_dir/rewrite-license" "$license"
  fi
  [[ "$verdict" == PASS ]]
}

staged_output() {
  case "$1" in
    fv01-observed-legacy-snapshot) printf '%s' "$stage_dir/fv01-observed.nul" ;;
    fv01-raw-result) printf '%s' "$stage_dir/fv01-raw.txt" ;;
    fv02-raw-result) printf '%s' "$stage_dir/fv02-raw.txt" ;;
    fv03-whole-tree-raw-result) printf '%s' "$stage_dir/fv03-raw.txt" ;;
    fv04-raw-result) printf '%s' "$stage_dir/fv04-raw.txt" ;;
    fv05-migration-adr-audit-evidence) printf '%s' "$stage_dir/fv05.json" ;;
    fv06-c2-provenance-evidence) printf '%s' "$stage_dir/fv06.json" ;;
    fv07-raw-result) printf '%s' "$stage_dir/fv07-raw.txt" ;;
    fv08-dependency-evidence) printf '%s' "$stage_dir/fv08-dependency.txt" ;;
    fv08-license-evidence) printf '%s' "$stage_dir/fv08-license.txt" ;;
    fv08-log-gitleaks-evidence) printf '%s' "$stage_dir/fv08-sensitive.txt" ;;
    linux-replay-receipt|aarch64-linux-replay-receipt|darwin-replay-receipt) printf '%s' '' ;;
    *) reject "no staged producer is defined for $1" ;;
  esac
}

mark_staged_failure() {
  local fv="$1" id staged rewritten
  while IFS= read -r id; do
    staged="$(staged_output "$id")"
    [[ -n "$staged" && -f "$staged" ]] || continue
    case "$id" in
      fv05-migration-adr-audit-evidence|fv06-c2-provenance-evidence)
        rewritten="$stage_dir/rewrite-${id}.json"
        jq -cS '.verdict = "FAIL"' "$staged" >"$rewritten"
        mv -- "$rewritten" "$staged" ;;
      fv01-observed-legacy-snapshot) ;;
      *)
        rewritten="$stage_dir/rewrite-${id}.txt"
        head -n -1 "$staged" >"$rewritten"
        printf '%s\n' 'VERDICT: FAIL' >>"$rewritten"
        mv -- "$rewritten" "$staged" ;;
    esac
  done < <(jq -r --arg fv "$fv" '.final_tree_record_protocol.fv_output_sets[$fv][]' "$manifest")
}

publish_current_outputs() {
  local fv="$1" id staged relative
  while IFS= read -r id; do
    relative="$(output_path "$id")"
    staged="$(staged_output "$id")"
    if [[ -z "$staged" ]]; then
      # FV07 replay receipts are created exclusively by the descriptor owner.
      validate_output "$id"
    else
      exclusive_publish "$staged" "$relative" "$id"
    fi
  done < <(jq -r --arg fv "$fv" '.final_tree_record_protocol.fv_output_sets[$fv][]' "$manifest")
}

output_set_hashes() {
  local fv="$1" id relative sha verdict rows='[]'
  while IFS= read -r id; do
    relative="$(output_path "$id")"
    validate_output "$id"
    case "$id" in
      *-receipt) sha="$(jq -r '.receipt_sha256' "$repo_root/$relative")" ;;
      *) sha="$(sha256_file "$repo_root/$relative")" ;;
    esac
    verdict=PASS
    rows="$(jq -cnS --argjson rows "$rows" --arg input_id "$id" --arg path "$relative" --arg sha256 "$sha" --arg verdict "$verdict" '$rows + [{input_id:$input_id,path:$path,sha256:$sha256,verdict:$verdict}]')"
  done < <(jq -r --arg fv "$fv" '.final_tree_record_protocol.fv_output_sets[$fv][]' "$manifest")
  printf '%s\n' "$rows"
}

primary_result_hash() {
  local fv="$1" id
  case "$fv" in
    FV01) id=fv01-raw-result ;;
    FV02) id=fv02-raw-result ;;
    FV03) id=fv03-whole-tree-raw-result ;;
    FV04) id=fv04-raw-result ;;
    FV05) id=fv05-migration-adr-audit-evidence ;;
    FV06) id=fv06-c2-provenance-evidence ;;
    FV07) id=fv07-raw-result ;;
    FV08) id=fv08-dependency-evidence ;;
    *) reject "unknown FV primary evidence: $fv" ;;
  esac
  sha256_file "$repo_root/$(output_path "$id")"
}

primary_result_id() {
  case "$1" in
    FV01) printf '%s' fv01-raw-result ;;
    FV02) printf '%s' fv02-raw-result ;;
    FV03) printf '%s' fv03-whole-tree-raw-result ;;
    FV04) printf '%s' fv04-raw-result ;;
    FV05) printf '%s' fv05-migration-adr-audit-evidence ;;
    FV06) printf '%s' fv06-c2-provenance-evidence ;;
    FV07) printf '%s' fv07-raw-result ;;
    FV08) printf '%s' fv08-dependency-evidence ;;
    *) reject "unknown FV primary evidence: $1" ;;
  esac
}

make_final_handoff() {
  local fv verify step_rows='[]' equality='{}' map_equality='{}' output_rows='{}' raw_rows='{}' references='[]'
  local x_terminal x_replay a_terminal a_replay d_terminal d_replay x_id a_id d_id handoff_stage
  for fv in "${FV_ORDER[@]}"; do
    verify="$(verify_tree_snapshot "$fv")"
    step_rows="$(jq -cnS --argjson rows "$step_rows" --arg fv "$fv" --arg digest "$(jq -r '.step_input_digest' <<<"$verify")" '$rows + [{fv_id:$fv,step_input_digest:$digest}]')"
    equality="$(jq -cnS --argjson rows "$equality" --arg fv "$fv" '$rows + {($fv):true}')"
    map_equality="$(jq -cnS --argjson rows "$map_equality" --arg fv "$fv" '$rows + {($fv):true}')"
    output_rows="$(jq -cnS --argjson rows "$output_rows" --arg fv "$fv" --argjson values "$(output_set_hashes "$fv")" '$rows + {($fv):$values}')"
    raw_rows="$(jq -cnS --argjson rows "$raw_rows" --arg fv "$fv" --arg sha "$(primary_result_hash "$fv")" '$rows + {($fv):$sha}')"
    references="$(jq -cnS --argjson rows "$references" --arg fv "$fv" --arg path "$(output_path "$(primary_result_id "$fv")")" '$rows + [{fv_id:$fv,path:$path}]')"
  done
  verify="$(verify_tree_snapshot FV08)"
  x_terminal="$(output_path protocol-linux-terminal-record)"; x_replay="$(output_path linux-replay-receipt)"
  a_terminal="$(output_path protocol-aarch64-linux-terminal-record)"; a_replay="$(output_path aarch64-linux-replay-receipt)"
  d_terminal="$(output_path protocol-darwin-terminal-record)"; d_replay="$(output_path darwin-replay-receipt)"
  for path in "$x_terminal" "$x_replay" "$a_terminal" "$a_replay" "$d_terminal" "$d_replay"; do require_canonical_object "$repo_root/$path" "handoff input $path"; done
  x_id="$(jq -r '.target_descriptor_id' "$repo_root/$x_terminal")"; a_id="$(jq -r '.target_descriptor_id' "$repo_root/$a_terminal")"; d_id="$(jq -r '.target_descriptor_id' "$repo_root/$d_terminal")"
  handoff_stage="$stage_dir/final-handoff.json"
  jq -cnS \
    --arg assertion_digest "$assertion_digest" --arg source_snapshot "$(jq -r '.source_snapshot' <<<"$verify")" \
    --argjson gpe_map "$(jq -c '.gpe_artifact_hash_map' <<<"$verify")" --arg gpe_map_sha "$gpe_map_sha" \
    --arg repository_sha "$(jq -r '.repository_manifest_sha256' <<<"$verify")" --arg ignored_sha "$(jq -r '.ignored_input_inventory_sha256' <<<"$verify")" \
    --arg oracle_sha "$(jq -r '.input_coverage_oracle_sha256' <<<"$verify")" --arg source_hash_sha "$(jq -r '.source_snapshot_hash_artifact_sha256' <<<"$verify")" \
    --arg binding_sha "$(jq -r '.generated_evidence_binding_sha256' <<<"$verify")" \
    --arg input_sets_sha "$(jq -cS '.final_tree_record_protocol.fv_input_sets' "$manifest" | sha256_stream)" \
    --arg output_sets_sha "$(jq -cS '.final_tree_record_protocol.fv_output_sets' "$manifest" | sha256_stream)" \
    --argjson step_rows "$step_rows" --argjson equality "$equality" --argjson map_equality "$map_equality" \
    --argjson output_rows "$output_rows" --argjson raw_rows "$raw_rows" --arg expected_selector_sha "$(document_selector_sha)" \
    --arg observed_sha "$(sha256_file "$repo_root/$(output_path fv01-observed-legacy-snapshot)")" \
    --arg x_id "$x_id" --arg x_descriptor "$(jq -r '.descriptor_digest' "$repo_root/$x_terminal")" --arg x_terminal "$x_terminal" \
    --arg x_record "$(jq -r '.record_sha256' "$repo_root/$x_terminal")" --arg x_replay "$(jq -r '.receipt_sha256' "$repo_root/$x_replay")" \
    --arg a_id "$a_id" --arg a_descriptor "$(jq -r '.descriptor_digest' "$repo_root/$a_terminal")" --arg a_terminal "$a_terminal" \
    --arg a_record "$(jq -r '.record_sha256' "$repo_root/$a_terminal")" --arg a_replay "$(jq -r '.receipt_sha256' "$repo_root/$a_replay")" \
    --arg d_id "$d_id" --arg d_descriptor "$(jq -r '.descriptor_digest' "$repo_root/$d_terminal")" --arg d_terminal "$d_terminal" \
    --arg d_record "$(jq -r '.record_sha256' "$repo_root/$d_terminal")" --arg d_replay "$(jq -r '.receipt_sha256' "$repo_root/$d_replay")" \
    --argjson references "$references" '
    {aarch64_linux_descriptor_digest:$a_descriptor,aarch64_linux_record_sha256:$a_record,aarch64_linux_replay_receipt_sha256:$a_replay,aarch64_linux_target_descriptor_id:$a_id,aarch64_linux_terminal_path:$a_terminal,assertion_digest:$assertion_digest,darwin_descriptor_digest:$d_descriptor,darwin_record_sha256:$d_record,darwin_replay_receipt_sha256:$d_replay,darwin_target_descriptor_id:$d_id,darwin_terminal_path:$d_terminal,fv01_expected_comparison_verdict:"PASS",fv01_expected_selector_sha256:$expected_selector_sha,fv01_observed_legacy_snapshot_sha256:$observed_sha,fv01_fv08_gpe_map_pre_post_equality:$map_equality,fv01_fv08_ordered_step_input_digests:$step_rows,fv01_fv08_output_hashes_and_verdicts:$output_rows,fv01_fv08_pre_post_input_set_equality:$equality,fv01_fv08_raw_result_sha256:$raw_rows,fv_input_sets_canonical_sha256:$input_sets_sha,fv_output_sets_canonical_sha256:$output_sets_sha,generated_evidence_binding_sha256:$binding_sha,gpe_artifact_hash_map:$gpe_map,gpe_artifact_hash_map_sha256:$gpe_map_sha,ignored_input_inventory_sha256:$ignored_sha,input_coverage_oracle_sha256:$oracle_sha,linux_descriptor_digest:$x_descriptor,linux_record_sha256:$x_record,linux_replay_receipt_sha256:$x_replay,linux_target_descriptor_id:$x_id,linux_terminal_path:$x_terminal,no_mutation_declarations:["no tag or publication","no deployment or production access","no SCM mutation"],per_step_exit_evidence_references:$references,repository_manifest_sha256:$repository_sha,source_snapshot:$source_snapshot,source_snapshot_hash_artifact_sha256:$source_hash_sha}' >"$handoff_stage"
  require_canonical_object "$handoff_stage" 'staged final handoff'
  jq -e --argjson fields "$(jq -c '.operator_surfaces.final_verify.handoff_fields' "$manifest")" '((keys|sort) == ($fields|sort)) and (has("handoff_sha256")|not)' "$handoff_stage" >/dev/null || reject 'final handoff field set differs from frozen protocol'
  exclusive_publish "$handoff_stage" "$HANDOFF_REL" 'R25 final handoff'
  require_canonical_object "$repo_root/$HANDOFF_REL" 'published R25 final handoff'
}

(( $# == 2 )) || usage
readonly digest_arg="$1"
readonly external_selector="$2"
case "$external_selector" in
  fv01) internal_fv=FV01; current_index=0 ;;
  fv02) internal_fv=FV02; current_index=1 ;;
  fv03) internal_fv=FV03; current_index=2 ;;
  fv04) internal_fv=FV04; current_index=3 ;;
  fv05) internal_fv=FV05; current_index=4 ;;
  fv06) internal_fv=FV06; current_index=5 ;;
  fv07) internal_fv=FV07; current_index=6 ;;
  fv08) internal_fv=FV08; current_index=7 ;;
  *) usage ;;
esac
readonly internal_fv current_index
verify_bootstrap "$digest_arg"
verify_manifest_output_paths
verify_output_sequence "$current_index"

stage_dir="$(mktemp -d "$repo_root/.agents/results/.task-13-r25-${external_selector}-stage.XXXXXX")" || reject 'cannot create same-filesystem FV staging directory'
cleanup_stage() { [[ -z "$stage_dir" || ! -d "$stage_dir" ]] || rm -rf -- "$stage_dir"; }
trap cleanup_stage EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

pre_state="$(verify_tree_snapshot "$internal_fv")"
step_source="$(jq -er '.source_snapshot' <<<"$pre_state")"
step_status=0
set +e
case "$internal_fv" in
  FV01) run_fv01 ;;
  FV02) run_fv02 ;;
  FV03) run_fv03 ;;
  FV04) run_fv04 ;;
  FV05) run_fv05 "$step_source" ;;
  FV06) run_fv06 "$step_source" ;;
  FV07) run_fv07 ;;
  FV08) run_fv08 ;;
esac
step_status=$?
set -e
post_state="$(verify_tree_snapshot "$internal_fv")"
if [[ "$pre_state" != "$post_state" ]]; then
  mark_staged_failure "$internal_fv"
  publish_current_outputs "$internal_fv"
  printf '%s\n' 'error: FV pre/post input set, source snapshot, or GPE map drifted; immutable typed evidence was retained' >&2
  exit 1
fi
if (( step_status != 0 )); then
  mark_staged_failure "$internal_fv"
  publish_current_outputs "$internal_fv"
  printf 'error: %s dispatch or audit failed; immutable typed evidence was retained\n' "$internal_fv" >&2
  exit "$step_status"
fi
publish_current_outputs "$internal_fv"
while IFS= read -r output_id; do validate_output "$output_id"; done < <(jq -r --arg fv "$internal_fv" '.final_tree_record_protocol.fv_output_sets[$fv][]' "$manifest")
if [[ "$internal_fv" == FV08 ]]; then make_final_handoff; fi
printf 'FV_ID: %s\nSTEP_INPUT_DIGEST: %s\nSOURCE_SNAPSHOT: %s\nGPE_ARTIFACT_HASH_MAP_SHA256: %s\nVERDICT: PASS\n' \
  "$internal_fv" "$(jq -r '.step_input_digest' <<<"$post_state")" "$step_source" "$gpe_map_sha"
