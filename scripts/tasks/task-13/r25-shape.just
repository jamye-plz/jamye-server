set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set positional-arguments

[private]
require-nix-shell:
    @[[ -n "${IN_NIX_SHELL:-}" ]] || { printf '%s\n' 'error: enter the repository Nix devShell before running task-13' >&2; exit 2; }

# R25 intentionally has no set working-directory declaration.  The copied
# guard supplies the only verified working directory and selected Justfile.
aarch64-linux-shape-r25-immutable: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    readonly generation_id='task13-r25-g0-f13-trust-root-cross-binding-20260907'
    [[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation_id" ]] || { printf '%s\n' 'error: active R25 shape requires guarded generation selection' >&2; exit 2; }
    [[ "${TASK13_ASSERTION_DIGEST:-}" =~ ^[0-9a-f]{64}$ ]] || { printf '%s\n' 'error: active R25 shape guard digest receipt is missing' >&2; exit 2; }
    [[ "${TASK13_R25_SELECTED_JUSTFILE_COPY:-}" == /* && -f "${TASK13_R25_SELECTED_JUSTFILE_COPY}" && ! -L "${TASK13_R25_SELECTED_JUSTFILE_COPY}" && "${TASK13_R25_SELECTED_JUSTFILE_SHA256:-}" =~ ^[0-9a-f]{64}$ && "${TASK13_R25_SELECTED_JUSTFILE_IDENTITY:-}" =~ ^[0-9]+:[0-9]+$ ]] || { printf '%s\n' 'error: selected R25 Justfile receipt is malformed' >&2; exit 2; }
    [[ "$(stat -c '%a' -- "$TASK13_R25_SELECTED_JUSTFILE_COPY")" == 400 && "$(stat -c '%d:%i' -- "$TASK13_R25_SELECTED_JUSTFILE_COPY")" == "$TASK13_R25_SELECTED_JUSTFILE_IDENTITY" && "$(sha256sum "$TASK13_R25_SELECTED_JUSTFILE_COPY" | awk '{print $1}')" == "$TASK13_R25_SELECTED_JUSTFILE_SHA256" && "${TASK13_BOUND_JUSTFILE:-}" == "$TASK13_R25_SELECTED_JUSTFILE_COPY" && "${TASK13_BOUND_JUSTFILE_SHA256:-}" == "$TASK13_R25_SELECTED_JUSTFILE_SHA256" && "${TASK13_BOUND_JUSTFILE_IDENTITY:-}" == "$TASK13_R25_SELECTED_JUSTFILE_IDENTITY" ]] || { printf '%s\n' 'error: selected R25 Justfile changed after guard validation' >&2; exit 2; }
    [[ "${TASK13_FLAKE_SOURCE_SNAPSHOT:-}" == /nix/store/* && -d "$TASK13_FLAKE_SOURCE_SNAPSHOT" ]] || { printf '%s\n' 'error: active R25 shape snapshot receipt is missing or invalid' >&2; exit 2; }
    [[ "$(pwd -P)" == "$TASK13_FLAKE_SOURCE_SNAPSHOT" ]] || { printf '%s\n' 'error: active R25 shape is not running from the guarded snapshot' >&2; exit 2; }
    [[ "$(sha256sum tests/nix/task-13-assertion-manifest-r25-g0-f13.json | awk '{print $1}')" == "$TASK13_ASSERTION_DIGEST" ]] || { printf '%s\n' 'error: active R25 manifest changed after guarded dispatch' >&2; exit 2; }
    exec bash tests/nix/flake-shape-aarch64-linux-r25.sh

[private]
coverage-r25-exec: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    readonly generation_id='task13-r25-g0-f13-trust-root-cross-binding-20260907'
    readonly assertion_digest="${TASK13_ASSERTION_DIGEST:-}"
    readonly expected_stream='bb77b0f773f339bc5f5767e57f96f6ad8352a299a53c1ff67f103595b4faa12f'
    readonly coverage_attribute='pkgs.cargo-llvm-cov'
    readonly nixpkgs_revision='56c02bc00adcf003215cc4bd996d6efaf4cff188'
    readonly coverage_target_directory='target/task-13-llvm-cov'
    readonly -a coverage_commands=(
      'CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov clean --workspace'
      'CARGO_NET_OFFLINE=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov --locked --offline --workspace --all-targets --no-report -- --test-threads=1'
      'CARGO_NET_OFFLINE=true JAMYE_ENABLE_DEV_FIXTURES=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov --locked --offline --workspace --all-targets --all-features --no-report -- --test-threads=1'
      'CARGO_NET_OFFLINE=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov report --locked --offline --json --summary-only --fail-under-lines 80'
    )
    invalid() { printf 'error: %s\n' "$1" >&2; exit 2; }
    verify_bound_justfile() {
      local selected="${TASK13_R25_SELECTED_JUSTFILE_COPY:-}" selected_sha="${TASK13_R25_SELECTED_JUSTFILE_SHA256:-}" selected_identity="${TASK13_R25_SELECTED_JUSTFILE_IDENTITY:-}"
      [[ "$selected" == /* && -f "$selected" && ! -L "$selected" && "$selected_sha" =~ ^[0-9a-f]{64}$ && "$selected_identity" =~ ^[0-9]+:[0-9]+$ ]] || invalid 'R25 selected Justfile receipt is malformed'
      [[ "$(stat -c '%a' -- "$selected")" == 400 && "$(stat -c '%d:%i' -- "$selected")" == "$selected_identity" && "$(sha256sum "$selected" | awk '{print $1}')" == "$selected_sha" ]] || invalid 'R25 selected Justfile changed'
      [[ "${TASK13_BOUND_JUSTFILE:-}" == "$selected" && "${TASK13_BOUND_JUSTFILE_SHA256:-}" == "$selected_sha" && "${TASK13_BOUND_JUSTFILE_IDENTITY:-}" == "$selected_identity" ]] || invalid 'generic Justfile receipts are not selected-copy aliases'
    }
    resolve_store_command() {
      local name="$1" resolved physical
      resolved="$(command -v "$name")" || invalid "required coverage command is unavailable: ${name}"
      physical="$(realpath -- "$resolved")" || invalid "required coverage command cannot be resolved: ${name}"
      [[ "$physical" == /nix/store/*/bin/"$name" && -x "$physical" ]] || invalid "required coverage command is outside pinned Nix store: ${name}"
      printf '%s' "$physical"
    }
    [[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation_id" && "$assertion_digest" =~ ^[0-9a-f]{64}$ ]] || invalid 'coverage requires guarded R25 generation and digest'
    [[ "$(printf '%s\n' "${coverage_commands[@]}" | sha256sum | awk '{print $1}')" == "$expected_stream" ]] || invalid 'coverage command stream differs from frozen R25 authority'
    verify_bound_justfile
    [[ -f .env.local && ! -L .env.local ]] || invalid '.env.local is missing; run the task-1 local-env-create card'
    while IFS='=' read -r key value; do [[ -z "$key" || "$key" == \#* ]] && continue; [[ "$key" =~ ^[A-Z][A-Z0-9_]*$ ]] || invalid "invalid key in .env.local: ${key}"; export "$key=$value"; done < .env.local
    rustc_path="$(resolve_store_command rustc)"; cargo_path="$(resolve_store_command cargo)"; cov_path="$(resolve_store_command cargo-llvm-cov)"
    [[ "$($rustc_path --version | awk '{print $2}')" == 1.98.0 ]] || invalid 'active Rust release differs from frozen R25 coverage authority'
    [[ "$($cov_path llvm-cov --version)" == 'cargo-llvm-cov 0.9.0' ]] || invalid 'cargo-llvm-cov version differs from frozen R25 coverage authority'
    rust_host="$($rustc_path -vV | sed -n 's/^host: //p')"; rust_sysroot="$(realpath -- "$($rustc_path --print sysroot)")"
    llvm_cov="$rust_sysroot/lib/rustlib/$rust_host/bin/llvm-cov"; llvm_profdata="$rust_sysroot/lib/rustlib/$rust_host/bin/llvm-profdata"
    [[ -x "$llvm_cov" && -x "$llvm_profdata" ]] || invalid 'llvm-tools-preview is missing from pinned Rust toolchain'
    export PATH="$(dirname -- "$rustc_path"):$(dirname -- "$cargo_path"):$(dirname -- "$cov_path"):${PATH:-}"
    printf 'COVERAGE_TOOL_ATTRIBUTE: %s\nCOVERAGE_NIXPKGS_REVISION: %s\nCOVERAGE_TOOL_STORE_PATH: %s\nCOVERAGE_TOOL_VERSION: %s\nCOVERAGE_RUST_VERSION: %s\nCOVERAGE_LLVM_COV_PATH: %s\nCOVERAGE_LLVM_PROFDATA_PATH: %s\nCOVERAGE_TARGET_DIRECTORY: %s\n' "$coverage_attribute" "$nixpkgs_revision" "${cov_path%/bin/cargo-llvm-cov}" "$($cov_path llvm-cov --version)" "$($rustc_path --version)" "$llvm_cov" "$llvm_profdata" "$coverage_target_directory"
    CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov clean --workspace
    CARGO_NET_OFFLINE=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov --locked --offline --workspace --all-targets --no-report -- --test-threads=1
    CARGO_NET_OFFLINE=true JAMYE_ENABLE_DEV_FIXTURES=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov --locked --offline --workspace --all-targets --all-features --no-report -- --test-threads=1
    report_file="target/task-13-coverage-evidence/${assertion_digest}/coverage-summary.json"; mkdir -p -- "${report_file%/*}"
    CARGO_NET_OFFLINE=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov report --locked --offline --json --summary-only --fail-under-lines 80 >"$report_file"
    jq -e '(.data|length)==1 and (.data[0].totals.lines.percent|type)=="number" and .data[0].totals.lines.percent >= 80' "$report_file" >/dev/null || invalid 'coverage JSON summary is missing or below frozen 80 percent threshold'
    printf '%s\n' 'COVERAGE_JSON_BEGIN'; cat -- "$report_file"; printf '%s\n' 'COVERAGE_JSON_END'
    printf 'COVERAGE_HUMAN_SUMMARY: line_percent=%s threshold_percent=80 result=met\n' "$(jq -er '.data[0].totals.lines.percent' "$report_file")"
    printf '%s\n' 'PASS: line coverage at or above 80 percent with all targets/all features'
    printf '%s\n' 'SUMMARY: 1 PASS / 0 FAIL / 0 SKIP'
    printf '%s\n' 'DISPOSITION: GREEN (zero named failures)'

[private]
module-eval-r25-exec phase: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    readonly generation_id='task13-r25-g0-f13-trust-root-cross-binding-20260907'
    invalid() { printf 'error: %s\n' "$1" >&2; exit 2; }
    [[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation_id" && "${TASK13_ASSERTION_DIGEST:-}" =~ ^[0-9a-f]{64}$ ]] || invalid 'module evaluation requires guarded R25 generation and digest'
    [[ "$phase" == s2-skeleton || "$phase" == s4-green || "$phase" == s5-final ]] || invalid 'module evaluation phase is not allowlisted'
    [[ "${TASK13_R25_SELECTED_JUSTFILE_COPY:-}" == /* && -f "${TASK13_R25_SELECTED_JUSTFILE_COPY}" && ! -L "${TASK13_R25_SELECTED_JUSTFILE_COPY}" && "${TASK13_R25_SELECTED_JUSTFILE_SHA256:-}" =~ ^[0-9a-f]{64}$ && "${TASK13_R25_SELECTED_JUSTFILE_IDENTITY:-}" =~ ^[0-9]+:[0-9]+$ ]] || invalid 'module selected R25 Justfile receipt is malformed'
    [[ "$(stat -c '%a' -- "$TASK13_R25_SELECTED_JUSTFILE_COPY")" == 400 && "$(stat -c '%d:%i' -- "$TASK13_R25_SELECTED_JUSTFILE_COPY")" == "$TASK13_R25_SELECTED_JUSTFILE_IDENTITY" && "$(sha256sum "$TASK13_R25_SELECTED_JUSTFILE_COPY" | awk '{print $1}')" == "$TASK13_R25_SELECTED_JUSTFILE_SHA256" && "${TASK13_BOUND_JUSTFILE:-}" == "$TASK13_R25_SELECTED_JUSTFILE_COPY" && "${TASK13_BOUND_JUSTFILE_SHA256:-}" == "$TASK13_R25_SELECTED_JUSTFILE_SHA256" && "${TASK13_BOUND_JUSTFILE_IDENTITY:-}" == "$TASK13_R25_SELECTED_JUSTFILE_IDENTITY" ]] || invalid 'module selected R25 Justfile changed after guard validation'
    repo_root="${TASK13_R25_REPO_ROOT:-}"; manifest_copy="${TASK13_R25_MANIFEST_COPY:-}"
    [[ "$repo_root" == /* && -d "$repo_root" && "$(pwd -P)" == "$(cd -P -- "$repo_root" && pwd -P)" ]] || invalid 'module evaluation live repository receipt is malformed'
    [[ "$manifest_copy" == /* && -f "$manifest_copy" && ! -L "$manifest_copy" && "$(stat -c '%a' -- "$manifest_copy")" == 400 && "$(sha256sum "$manifest_copy" | awk '{print $1}')" == "${TASK13_ASSERTION_DIGEST}" ]] || invalid 'module evaluation manifest-copy receipt is malformed'
    case "$phase" in s2-skeleton) projection=s2_skeleton_red;; s4-green) projection=s4_green;; s5-final) projection=s5_final_green;; esac
    evaluator="$repo_root/tests/nix/module-eval.nix"; [[ -f "$evaluator" && ! -L "$evaluator" ]] || invalid 'tests/nix/module-eval.nix is missing or linked'
    result="$(nix eval --impure --json --no-write-lock-file --file "$evaluator" --argstr phase "$phase" --argstr repoRoot "$repo_root")" || exit "$?"
    expected="$(jq -ce --arg projection "$projection" '.module_eval[$projection]' "$manifest_copy")" || invalid 'module evaluation projection is missing from active manifest'
    jq -e --argjson expected "$expected" 'keys == ["results"] and (.results|type=="array" and length==18) and (all(.results[]; keys == ["detail","name","status"] and (.status=="PASS" or .status=="FAIL" or .status=="SKIP"))) and ([.results[]|select(.status=="PASS")|.name] == $expected.pass) and ([.results[]|select(.status=="FAIL")|.name] == $expected.fail) and ([.results[]|select(.status=="SKIP")|"\(.name): \(.detail)"] == $expected.skip)' <<<"$result" >/dev/null || invalid 'module evaluation result differs from frozen phase oracle'
    jq -r '.results[] | if .detail == "" then "\(.status): \(.name)" else "\(.status): \(.name) — \(.detail)" end' <<<"$result"
    pass_count="$(jq '[.results[]|select(.status=="PASS")]|length' <<<"$result")"; fail_count="$(jq '[.results[]|select(.status=="FAIL")]|length' <<<"$result")"; skip_count="$(jq '[.results[]|select(.status=="SKIP")]|length' <<<"$result")"
    printf 'SUMMARY: %s PASS / %s FAIL / %s SKIP\n' "$pass_count" "$fail_count" "$skip_count"
    [[ "$phase" != s2-skeleton ]] || { printf '%s\n' 'DISPOSITION: BASELINE_RED (twelve named failures)'; exit 1; }
    printf '%s\n' 'DISPOSITION: GREEN (zero named failures)'
    if [[ "$phase" == s5-final ]]; then
      helper="${TASK13_R25_FINAL_TREE_HELPER_COPY:-}"; helper_sha="${TASK13_R25_FINAL_TREE_HELPER_SHA256:-}"
      [[ "$helper" == /* && -f "$helper" && ! -L "$helper" && "$(stat -c '%a' -- "$helper")" == 400 && "$helper_sha" =~ ^[0-9a-f]{64}$ && "$(sha256sum "$helper"|awk '{print $1}')" == "$helper_sha" ]] || invalid 'final-tree helper copy is malformed'
      bash "$helper" prepare-snapshot "${TASK13_ASSERTION_DIGEST}"
    fi

[private]
linux-builder-r25-exec gpe_map_digest replay_mode='': require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    generation='task13-r25-g0-f13-trust-root-cross-binding-20260907'
    [[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation" && "${TASK13_ASSERTION_DIGEST:-}" =~ ^[0-9a-f]{64}$ && "$gpe_map_digest" =~ ^[0-9a-f]{64}$ && ( -z "$replay_mode" || "$replay_mode" == fv07-replay ) ]] || { printf '%s\n' 'error: x86_64-linux descriptor arguments are malformed' >&2; exit 2; }
    [[ "${TASK13_R25_SELECTED_JUSTFILE_COPY:-}" == /* && -f "${TASK13_R25_SELECTED_JUSTFILE_COPY}" && ! -L "${TASK13_R25_SELECTED_JUSTFILE_COPY}" && "${TASK13_R25_SELECTED_JUSTFILE_SHA256:-}" =~ ^[0-9a-f]{64}$ && "$(stat -c '%a' -- "$TASK13_R25_SELECTED_JUSTFILE_COPY")" == 400 && "$(stat -c '%d:%i' -- "$TASK13_R25_SELECTED_JUSTFILE_COPY")" == "${TASK13_R25_SELECTED_JUSTFILE_IDENTITY:-}" && "$(sha256sum "$TASK13_R25_SELECTED_JUSTFILE_COPY"|awk '{print $1}')" == "${TASK13_R25_SELECTED_JUSTFILE_SHA256:-}" && "${TASK13_BOUND_JUSTFILE:-}" == "$TASK13_R25_SELECTED_JUSTFILE_COPY" && "${TASK13_BOUND_JUSTFILE_SHA256:-}" == "${TASK13_R25_SELECTED_JUSTFILE_SHA256:-}" && "${TASK13_BOUND_JUSTFILE_IDENTITY:-}" == "${TASK13_R25_SELECTED_JUSTFILE_IDENTITY:-}" ]] || { printf '%s\n' 'error: x86_64-linux selected Justfile receipt is malformed' >&2; exit 2; }
    helper="${TASK13_R25_FINAL_TREE_HELPER_COPY:-}"; sha="${TASK13_R25_FINAL_TREE_HELPER_SHA256:-}"
    [[ "$helper" == /* && -f "$helper" && ! -L "$helper" && "$(stat -c '%a' -- "$helper")" == 400 && "$sha" =~ ^[0-9a-f]{64}$ && "$(sha256sum "$helper"|awk '{print $1}')" == "$sha" ]] || { printf '%s\n' 'error: x86_64-linux descriptor helper receipt is malformed' >&2; exit 2; }
    export TASK13_R25_GPE_MAP_SHA256="$gpe_map_digest"
    exec bash "$helper" run-descriptor "${TASK13_ASSERTION_DIGEST}" x86_64-linux "$gpe_map_digest" "${replay_mode:-first}"

[private]
aarch64-linux-builder-r25-exec gpe_map_digest replay_mode='': require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    generation='task13-r25-g0-f13-trust-root-cross-binding-20260907'
    [[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation" && "${TASK13_ASSERTION_DIGEST:-}" =~ ^[0-9a-f]{64}$ && "$gpe_map_digest" =~ ^[0-9a-f]{64}$ && ( -z "$replay_mode" || "$replay_mode" == fv07-replay ) ]] || { printf '%s\n' 'error: aarch64-linux descriptor arguments are malformed' >&2; exit 2; }
    [[ "${TASK13_R25_SELECTED_JUSTFILE_COPY:-}" == /* && -f "${TASK13_R25_SELECTED_JUSTFILE_COPY}" && ! -L "${TASK13_R25_SELECTED_JUSTFILE_COPY}" && "${TASK13_R25_SELECTED_JUSTFILE_SHA256:-}" =~ ^[0-9a-f]{64}$ && "$(stat -c '%a' -- "$TASK13_R25_SELECTED_JUSTFILE_COPY")" == 400 && "$(stat -c '%d:%i' -- "$TASK13_R25_SELECTED_JUSTFILE_COPY")" == "${TASK13_R25_SELECTED_JUSTFILE_IDENTITY:-}" && "$(sha256sum "$TASK13_R25_SELECTED_JUSTFILE_COPY"|awk '{print $1}')" == "${TASK13_R25_SELECTED_JUSTFILE_SHA256:-}" && "${TASK13_BOUND_JUSTFILE:-}" == "$TASK13_R25_SELECTED_JUSTFILE_COPY" && "${TASK13_BOUND_JUSTFILE_SHA256:-}" == "${TASK13_R25_SELECTED_JUSTFILE_SHA256:-}" && "${TASK13_BOUND_JUSTFILE_IDENTITY:-}" == "${TASK13_R25_SELECTED_JUSTFILE_IDENTITY:-}" ]] || { printf '%s\n' 'error: aarch64-linux selected Justfile receipt is malformed' >&2; exit 2; }
    helper="${TASK13_R25_FINAL_TREE_HELPER_COPY:-}"; sha="${TASK13_R25_FINAL_TREE_HELPER_SHA256:-}"
    [[ "$helper" == /* && -f "$helper" && ! -L "$helper" && "$(stat -c '%a' -- "$helper")" == 400 && "$sha" =~ ^[0-9a-f]{64}$ && "$(sha256sum "$helper"|awk '{print $1}')" == "$sha" ]] || { printf '%s\n' 'error: aarch64-linux descriptor helper receipt is malformed' >&2; exit 2; }
    export TASK13_R25_GPE_MAP_SHA256="$gpe_map_digest"
    exec bash "$helper" run-descriptor "${TASK13_ASSERTION_DIGEST}" aarch64-linux "$gpe_map_digest" "${replay_mode:-first}"

[private]
cross-system-verify-r25-exec gpe_map_digest replay_mode='': require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    generation='task13-r25-g0-f13-trust-root-cross-binding-20260907'
    [[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation" && "${TASK13_ASSERTION_DIGEST:-}" =~ ^[0-9a-f]{64}$ && "$gpe_map_digest" =~ ^[0-9a-f]{64}$ && ( -z "$replay_mode" || "$replay_mode" == fv07-replay ) ]] || { printf '%s\n' 'error: aarch64-darwin descriptor arguments are malformed' >&2; exit 2; }
    [[ "${TASK13_R25_SELECTED_JUSTFILE_COPY:-}" == /* && -f "${TASK13_R25_SELECTED_JUSTFILE_COPY}" && ! -L "${TASK13_R25_SELECTED_JUSTFILE_COPY}" && "${TASK13_R25_SELECTED_JUSTFILE_SHA256:-}" =~ ^[0-9a-f]{64}$ && "$(stat -c '%a' -- "$TASK13_R25_SELECTED_JUSTFILE_COPY")" == 400 && "$(stat -c '%d:%i' -- "$TASK13_R25_SELECTED_JUSTFILE_COPY")" == "${TASK13_R25_SELECTED_JUSTFILE_IDENTITY:-}" && "$(sha256sum "$TASK13_R25_SELECTED_JUSTFILE_COPY"|awk '{print $1}')" == "${TASK13_R25_SELECTED_JUSTFILE_SHA256:-}" && "${TASK13_BOUND_JUSTFILE:-}" == "$TASK13_R25_SELECTED_JUSTFILE_COPY" && "${TASK13_BOUND_JUSTFILE_SHA256:-}" == "${TASK13_R25_SELECTED_JUSTFILE_SHA256:-}" && "${TASK13_BOUND_JUSTFILE_IDENTITY:-}" == "${TASK13_R25_SELECTED_JUSTFILE_IDENTITY:-}" ]] || { printf '%s\n' 'error: aarch64-darwin selected Justfile receipt is malformed' >&2; exit 2; }
    helper="${TASK13_R25_FINAL_TREE_HELPER_COPY:-}"; sha="${TASK13_R25_FINAL_TREE_HELPER_SHA256:-}"
    [[ "$helper" == /* && -f "$helper" && ! -L "$helper" && "$(stat -c '%a' -- "$helper")" == 400 && "$sha" =~ ^[0-9a-f]{64}$ && "$(sha256sum "$helper"|awk '{print $1}')" == "$sha" ]] || { printf '%s\n' 'error: aarch64-darwin descriptor helper receipt is malformed' >&2; exit 2; }
    export TASK13_R25_GPE_MAP_SHA256="$gpe_map_digest"
    exec bash "$helper" run-descriptor "${TASK13_ASSERTION_DIGEST}" aarch64-darwin "$gpe_map_digest" "${replay_mode:-first}"

[private]
final-verify-r25-exec fv_id gpe_map_digest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    readonly generation_id='task13-r25-g0-f13-trust-root-cross-binding-20260907'
    [[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation_id" && "${TASK13_ASSERTION_DIGEST:-}" =~ ^[0-9a-f]{64}$ && "$gpe_map_digest" =~ ^[0-9a-f]{64}$ ]] || { printf '%s\n' 'error: final verification digest arguments are malformed' >&2; exit 2; }
    [[ "${TASK13_R25_GPE_MAP_SHA256:-}" == "$gpe_map_digest" ]] || { printf '%s\n' 'error: final verification GPE receipt does not match requested digest' >&2; exit 2; }
    [[ "${TASK13_R25_SELECTED_JUSTFILE_COPY:-}" == /* && -f "${TASK13_R25_SELECTED_JUSTFILE_COPY}" && ! -L "${TASK13_R25_SELECTED_JUSTFILE_COPY}" && "${TASK13_R25_SELECTED_JUSTFILE_SHA256:-}" =~ ^[0-9a-f]{64}$ && "$(stat -c '%a' -- "$TASK13_R25_SELECTED_JUSTFILE_COPY")" == 400 && "$(stat -c '%d:%i' -- "$TASK13_R25_SELECTED_JUSTFILE_COPY")" == "${TASK13_R25_SELECTED_JUSTFILE_IDENTITY:-}" && "$(sha256sum "$TASK13_R25_SELECTED_JUSTFILE_COPY"|awk '{print $1}')" == "${TASK13_R25_SELECTED_JUSTFILE_SHA256:-}" && "${TASK13_BOUND_JUSTFILE:-}" == "$TASK13_R25_SELECTED_JUSTFILE_COPY" && "${TASK13_BOUND_JUSTFILE_SHA256:-}" == "${TASK13_R25_SELECTED_JUSTFILE_SHA256:-}" && "${TASK13_BOUND_JUSTFILE_IDENTITY:-}" == "${TASK13_R25_SELECTED_JUSTFILE_IDENTITY:-}" ]] || { printf '%s\n' 'error: final verification selected Justfile receipt is malformed' >&2; exit 2; }
    case "$fv_id" in fv01|fv02|fv03|fv04|fv05|fv06|fv07|fv08) ;; *) printf '%s\n' 'error: final verification selector is not allowlisted' >&2; exit 2;; esac
    final_helper="${TASK13_R25_FINAL_VERIFY_HELPER_COPY:-}"; final_sha="${TASK13_R25_FINAL_VERIFY_HELPER_SHA256:-}"; tree_helper="${TASK13_R25_FINAL_TREE_HELPER_COPY:-}"; tree_sha="${TASK13_R25_FINAL_TREE_HELPER_SHA256:-}"
    for tuple in "final:$final_helper:$final_sha" "tree:$tree_helper:$tree_sha"; do label="${tuple%%:*}"; rest="${tuple#*:}"; path="${rest%:*}"; sha="${rest##*:}"; [[ "$path" == /* && -f "$path" && ! -L "$path" && "$(stat -c '%a' -- "$path")" == 400 && "$sha" =~ ^[0-9a-f]{64}$ && "$(sha256sum "$path"|awk '{print $1}')" == "$sha" ]] || { printf 'error: %s helper receipt is malformed\n' "$label" >&2; exit 2; }; done
    export TASK13_R25_GPE_MAP_SHA256="$gpe_map_digest"
    exec bash "$final_helper" "${TASK13_ASSERTION_DIGEST}" "$fv_id"
