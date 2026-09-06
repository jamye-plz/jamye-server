set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set positional-arguments
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter the repository Nix devShell before running task-13' >&2; exit 2; fi

# Frozen t13-s2 nested implementation. The mutable Task-13 mod.just may add
# later sprint recipes, but the active guard dispatches this exact Justfile
# only from its verified immutable R21 source snapshot.
aarch64-linux-shape-r21-immutable: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    generation_id='task13-r21-g0-f9-frozen-coverage-entrypoint-20260907'
    [[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation_id" ]] || { printf '%s\n' 'error: active R21 shape requires guarded generation selection' >&2; exit 2; }
    [[ "${TASK13_ASSERTION_DIGEST:-}" =~ ^[0-9a-f]{64}$ ]] || { printf '%s\n' 'error: active R21 shape guard digest receipt is missing' >&2; exit 2; }
    [[ "${TASK13_FLAKE_SOURCE_SNAPSHOT:-}" == /nix/store/* && -d "$TASK13_FLAKE_SOURCE_SNAPSHOT" ]] || { printf '%s\n' 'error: active R21 shape snapshot receipt is missing or invalid' >&2; exit 2; }
    [[ "$(pwd -P)" == "$TASK13_FLAKE_SOURCE_SNAPSHOT" ]] || { printf '%s\n' 'error: active R21 shape is not running from the guarded snapshot' >&2; exit 2; }
    [[ "$(sha256sum tests/nix/task-13-assertion-manifest-r21-g0-f9.json | awk '{print $1}')" == "$TASK13_ASSERTION_DIGEST" ]] || { printf '%s\n' 'error: active R21 manifest changed after guarded dispatch' >&2; exit 2; }
    exec bash tests/nix/flake-shape-aarch64-linux-r21.sh

[private]
coverage-r21-exec: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail

    readonly generation_id='task13-r21-g0-f9-frozen-coverage-entrypoint-20260907'
    readonly assertion_digest="${TASK13_ASSERTION_DIGEST:-}"
    readonly expected_rust_version='1.98.0'
    readonly expected_coverage_version='0.9.0'
    readonly coverage_attribute='pkgs.cargo-llvm-cov'
    readonly nixpkgs_revision='56c02bc00adcf003215cc4bd996d6efaf4cff188'
    readonly coverage_target_directory='target/task-13-llvm-cov'
    readonly expected_coverage_stream_sha='bb77b0f773f339bc5f5767e57f96f6ad8352a299a53c1ff67f103595b4faa12f'
    readonly -a coverage_commands=(
      'CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov clean --workspace'
      'CARGO_NET_OFFLINE=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov --locked --offline --workspace --all-targets --no-report -- --test-threads=1'
      'CARGO_NET_OFFLINE=true JAMYE_ENABLE_DEV_FIXTURES=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov --locked --offline --workspace --all-targets --all-features --no-report -- --test-threads=1'
      'CARGO_NET_OFFLINE=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov report --locked --offline --json --summary-only --fail-under-lines 80'
    )
    readonly expected_integration_targets='account_deletion,architecture,auth,auth_logging,chatrooms,contract,core_schema,dev_fixtures,groups,media,messaging,notifications,platform,production_composition,profile,rate_limit,realtime,realtime_membership,topics'
    readonly frozen_target_inventory='lib jamye_server; bins api,worker,generate_contracts; integration binaries lexical-sorted account_deletion,architecture,auth,auth_logging,chatrooms,contract,core_schema,dev_fixtures,groups,media,messaging,notifications,platform,production_composition,profile,rate_limit,realtime,realtime_membership,topics; nested support files are not targets'

    invalid() {
      printf 'error: %s\n' "$1" >&2
      exit 2
    }

    verify_bound_justfile_copy() {
      local path="${TASK13_BOUND_JUSTFILE:-}"
      local expected_identity="${TASK13_BOUND_JUSTFILE_IDENTITY:-}"
      local expected_sha="${TASK13_BOUND_JUSTFILE_SHA256:-}"
      local directory basename physical_directory physical_path
      [[ "$path" == /* && "$expected_identity" =~ ^[0-9]+:[0-9]+$ && "$expected_sha" =~ ^[0-9a-f]{64}$ ]] || invalid 'coverage guard-bound Justfile receipt is malformed'
      directory="$(dirname -- "$path")"
      basename="$(basename -- "$path")"
      [[ -d "$directory" ]] || invalid 'coverage guard-bound Justfile directory is missing'
      physical_directory="$(cd -P -- "$directory" && pwd -P)" || invalid 'coverage guard-bound Justfile directory cannot be resolved'
      physical_path="${physical_directory}/${basename}"
      [[ "$physical_path" == "$path" && -f "$physical_path" && ! -L "$physical_path" ]] || invalid 'coverage guard-bound Justfile is missing, linked, or escaped'
      [[ "$(stat -c '%a' -- "$physical_path")" == 400 ]] || invalid 'coverage guard-bound Justfile is not read-only'
      [[ "$(stat -c '%d:%i' -- "$physical_path")" == "$expected_identity" ]] || invalid 'coverage guard-bound Justfile identity changed'
      [[ "$(sha256sum "$physical_path" | awk '{print $1}')" == "$expected_sha" ]] || invalid 'coverage guard-bound Justfile content changed'
    }

    resolve_store_command() {
      local name="$1"
      local output_variable="$2"
      local resolved physical
      resolved="$(command -v "$name")" || invalid "required coverage command is unavailable: ${name}"
      physical="$(realpath -- "$resolved")" || invalid "required coverage command cannot be resolved: ${name}"
      case "$physical" in
        /nix/store/*/bin/"$name") ;;
        *) invalid "required coverage command resolved outside its pinned Nix store output: ${name}" ;;
      esac
      [[ -x "$physical" ]] || invalid "required coverage command is not executable: ${name}"
      printf -v "$output_variable" '%s' "$physical"
    }

    verify_target_inventory() {
      local metadata_json="$1"
      local mode="$2"
      if ! jq -e \
        --arg integration_targets "$expected_integration_targets" '
          (.packages | length) == 1
          and (.workspace_members | length) == 1
          and .packages[0].name == "jamye-server"
          and (all(.packages[0].targets[];
            (.kind | length) == 1
            and (.kind[0] == "lib" or .kind[0] == "bin" or .kind[0] == "test")
            and ((.["required-features"] // []) | length) == 0
          ))
          and ([.packages[0].targets[] | select(.kind == ["lib"]) | .name] | sort | join(",")) == "jamye_server"
          and ([.packages[0].targets[] | select(.kind == ["bin"]) | .name] | sort | join(",")) == "api,generate_contracts,worker"
          and ([.packages[0].targets[] | select(.kind == ["test"]) | .name] | sort | join(",")) == $integration_targets
        ' <<<"$metadata_json" >/dev/null; then
        invalid "${mode} target inventory differs from the frozen R21 oracle"
      fi
    }

    [[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation_id" ]] || invalid 'coverage requires guarded R21 generation selection'
    [[ "$assertion_digest" =~ ^[0-9a-f]{64}$ ]] || invalid 'coverage requires the exact active R21 manifest digest'
    actual_coverage_stream_sha="$(printf '%s\n' "${coverage_commands[@]}" | sha256sum | awk '{print $1}')"
    [[ "$actual_coverage_stream_sha" == "$expected_coverage_stream_sha" ]] || invalid 'coverage command stream differs from frozen R21 authority'
    verify_bound_justfile_copy

    # Keep the environment loader inside the guard-bound adjacent Justfile
    # copy so the active R21 executor has no mutable auxiliary dependency.
    readonly local_env_file="$(pwd -P)/.env.local"
    [[ -f "$local_env_file" ]] || invalid '.env.local is missing; run the task-1 local-env-create card'
    while IFS='=' read -r key value; do
      [[ -z "$key" || "$key" == \#* ]] && continue
      [[ "$key" =~ ^[A-Z][A-Z0-9_]*$ ]] || invalid "invalid key in .env.local: ${key}"
      export "$key=$value"
    done < "$local_env_file"

    rustc_path=''
    cargo_path=''
    cargo_llvm_cov_path=''
    resolve_store_command rustc rustc_path
    resolve_store_command cargo cargo_path
    resolve_store_command cargo-llvm-cov cargo_llvm_cov_path

    actual_rust_version="$($rustc_path --version | awk '{print $2}')"
    [[ "$actual_rust_version" == "$expected_rust_version" ]] || invalid 'active Rust release differs from frozen R21 coverage authority'
    coverage_version_output="$("$cargo_llvm_cov_path" llvm-cov --version)"
    [[ "$coverage_version_output" == "cargo-llvm-cov ${expected_coverage_version}" ]] || invalid 'cargo-llvm-cov version differs from frozen R21 coverage authority'

    rust_host="$($rustc_path -vV | sed -n 's/^host: //p')"
    [[ "$rust_host" =~ ^[A-Za-z0-9_.-]+$ ]] || invalid 'active Rust host triple is malformed'
    rust_sysroot="$($rustc_path --print sysroot)"
    rust_sysroot="$(realpath -- "$rust_sysroot")" || invalid 'active Rust sysroot cannot be resolved'
    case "$rust_sysroot" in
      /nix/store/*) ;;
      *) invalid 'active Rust sysroot resolved outside the Nix store' ;;
    esac
    llvm_cov_path="$rust_sysroot/lib/rustlib/$rust_host/bin/llvm-cov"
    llvm_profdata_path="$rust_sysroot/lib/rustlib/$rust_host/bin/llvm-profdata"
    [[ -x "$llvm_cov_path" && -x "$llvm_profdata_path" ]] || invalid 'llvm-tools-preview is missing from the pinned Rust toolchain'
    llvm_cov_path="$(realpath -- "$llvm_cov_path")"
    llvm_profdata_path="$(realpath -- "$llvm_profdata_path")"
    case "$llvm_cov_path:$llvm_profdata_path" in
      /nix/store/*:/nix/store/*) ;;
      *) invalid 'llvm-tools-preview resolved outside the Nix store' ;;
    esac

    export PATH="$(dirname -- "$rustc_path"):$(dirname -- "$cargo_path"):$(dirname -- "$cargo_llvm_cov_path"):${PATH:-}"
    hash -r
    [[ "$(realpath -- "$(command -v rustc)")" == "$rustc_path" ]] || invalid 'rustc changed after coverage PATH binding'
    [[ "$(realpath -- "$(command -v cargo)")" == "$cargo_path" ]] || invalid 'cargo changed after coverage PATH binding'
    [[ "$(realpath -- "$(command -v cargo-llvm-cov)")" == "$cargo_llvm_cov_path" ]] || invalid 'cargo-llvm-cov changed after coverage PATH binding'

    default_metadata="$(CARGO_NET_OFFLINE=true "$cargo_path" metadata --locked --offline --no-deps --format-version 1)"
    fixture_metadata="$(CARGO_NET_OFFLINE=true JAMYE_ENABLE_DEV_FIXTURES=true "$cargo_path" metadata --locked --offline --no-deps --all-features --format-version 1)"
    verify_target_inventory "$default_metadata" default_mode
    verify_target_inventory "$fixture_metadata" fixture_all_features_mode

    cargo_llvm_cov_store_path="${cargo_llvm_cov_path%/bin/cargo-llvm-cov}"
    printf 'COVERAGE_TOOL_ATTRIBUTE: %s\n' "$coverage_attribute"
    printf 'COVERAGE_NIXPKGS_REVISION: %s\n' "$nixpkgs_revision"
    printf 'COVERAGE_TOOL_STORE_PATH: %s\n' "$cargo_llvm_cov_store_path"
    printf 'COVERAGE_TOOL_VERSION: %s\n' "$coverage_version_output"
    printf 'COVERAGE_RUST_VERSION: %s\n' "$($rustc_path --version)"
    printf 'COVERAGE_LLVM_COV_PATH: %s\n' "$llvm_cov_path"
    printf 'COVERAGE_LLVM_PROFDATA_PATH: %s\n' "$llvm_profdata_path"
    printf 'COVERAGE_TARGET_DIRECTORY: %s\n' "$coverage_target_directory"
    printf 'TARGET_INVENTORY: default_mode=%s\n' "$frozen_target_inventory"
    printf 'TARGET_INVENTORY: fixture_all_features_mode=%s\n' "$frozen_target_inventory"

    CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov clean --workspace

    readonly evidence_directory="target/task-13-coverage-evidence/${assertion_digest}"
    umask 077
    mkdir -p -- "$evidence_directory"
    printf '%s\n' "$default_metadata" >"$evidence_directory/default-target-metadata.json"
    printf '%s\n' "$fixture_metadata" >"$evidence_directory/fixture-all-features-target-metadata.json"

    if CARGO_NET_OFFLINE=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov --locked --offline --workspace --all-targets --no-report -- --test-threads=1 >"$evidence_directory/default-mode.log" 2>&1; then
      cat -- "$evidence_directory/default-mode.log"
    else
      status=$?
      cat -- "$evidence_directory/default-mode.log" >&2
      exit "$status"
    fi

    shopt -s globstar nullglob
    default_profiles=("$coverage_target_directory"/**/*.profraw)
    ((${#default_profiles[@]} > 0)) || invalid 'default coverage mode produced no retained raw profiles'
    printf '%s\n' "${default_profiles[@]}" >"$evidence_directory/default-profraw-files.txt"

    if CARGO_NET_OFFLINE=true JAMYE_ENABLE_DEV_FIXTURES=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov --locked --offline --workspace --all-targets --all-features --no-report -- --test-threads=1 >"$evidence_directory/fixture-all-features-mode.log" 2>&1; then
      cat -- "$evidence_directory/fixture-all-features-mode.log"
    else
      status=$?
      cat -- "$evidence_directory/fixture-all-features-mode.log" >&2
      exit "$status"
    fi

    retained_default_profiles=0
    for profile in "${default_profiles[@]}"; do
      [[ -f "$profile" ]] || invalid 'fixture coverage mode replaced a retained default raw profile'
      ((retained_default_profiles += 1))
    done
    all_profiles=("$coverage_target_directory"/**/*.profraw)
    ((${#all_profiles[@]} > ${#default_profiles[@]})) || invalid 'fixture coverage mode did not add retained raw profiles'
    printf '%s\n' "${all_profiles[@]}" >"$evidence_directory/combined-profraw-files.txt"
    printf 'COVERAGE_MODE_RETENTION: default_profiles=%s retained_after_fixture=%s combined_profiles=%s clean=once report=once\n' \
      "${#default_profiles[@]}" "$retained_default_profiles" "${#all_profiles[@]}"

    if CARGO_NET_OFFLINE=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov report --locked --offline --json --summary-only --fail-under-lines 80 >"$evidence_directory/coverage-summary.json" 2>"$evidence_directory/coverage-report.stderr.log"; then
      cat -- "$evidence_directory/coverage-report.stderr.log" >&2
    else
      status=$?
      cat -- "$evidence_directory/coverage-report.stderr.log" >&2
      [[ ! -s "$evidence_directory/coverage-summary.json" ]] || cat -- "$evidence_directory/coverage-summary.json" >&2
      exit "$status"
    fi

    jq -e '
      (.data | length) == 1
      and (.data[0].totals.lines.percent | type) == "number"
      and .data[0].totals.lines.percent >= 80
    ' "$evidence_directory/coverage-summary.json" >/dev/null || invalid 'coverage JSON summary is missing or below the frozen 80 percent threshold'
    coverage_percent="$(jq -er '.data[0].totals.lines.percent' "$evidence_directory/coverage-summary.json")"
    printf '%s\n' 'COVERAGE_JSON_BEGIN'
    cat -- "$evidence_directory/coverage-summary.json"
    printf '%s\n' 'COVERAGE_JSON_END'
    printf 'COVERAGE_HUMAN_SUMMARY: line_percent=%s threshold_percent=80 result=met\n' "$coverage_percent"
    printf 'COVERAGE_EVIDENCE_DIRECTORY: %s\n' "$evidence_directory"
    printf '%s\n' 'PASS: line coverage at or above 80 percent with all targets/all features'
    printf '%s\n' 'SUMMARY: 1 PASS / 0 FAIL / 0 SKIP'
    printf '%s\n' 'DISPOSITION: GREEN (zero named failures)'

[private]
module-eval-r21-exec phase: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail

    readonly generation_id='task13-r21-g0-f9-frozen-coverage-entrypoint-20260907'
    readonly assertion_digest="${TASK13_ASSERTION_DIGEST:-}"
    readonly manifest_copy="${TASK13_R21_MANIFEST_COPY:-}"
    readonly repo_root="${TASK13_R21_REPO_ROOT:-}"

    invalid() {
      printf 'error: %s\n' "$1" >&2
      exit 2
    }

    verify_helper_copy() {
      local helper_path="$1"
      local expected_sha="$2"
      local label="$3"
      local helper_directory helper_basename physical_directory physical_path
      [[ "$helper_path" == /* && "$expected_sha" =~ ^[0-9a-f]{64}$ ]] || invalid "${label} receipt is malformed"
      helper_directory="$(dirname -- "$helper_path")"
      helper_basename="$(basename -- "$helper_path")"
      [[ -d "$helper_directory" ]] || invalid "${label} directory is missing"
      physical_directory="$(cd -P -- "$helper_directory" && pwd -P)" || invalid "${label} directory cannot be resolved"
      physical_path="${physical_directory}/${helper_basename}"
      [[ "$physical_path" == "$helper_path" && -f "$physical_path" && ! -L "$physical_path" ]] || invalid "${label} is missing, linked, or escaped"
      [[ "$(stat -c '%a' -- "$physical_path")" == 400 ]] || invalid "${label} is not read-only"
      [[ "$(sha256sum "$physical_path" | awk '{print $1}')" == "$expected_sha" ]] || invalid "${label} content changed"
    }

    [[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation_id" ]] || invalid 'module evaluation requires guarded R21 generation selection'
    [[ "$assertion_digest" =~ ^[0-9a-f]{64}$ ]] || invalid 'module evaluation requires the exact active R21 manifest digest'
    [[ "$phase" == s2-skeleton || "$phase" == s4-green || "$phase" == s5-final ]] || invalid 'module evaluation phase is not allowlisted'
    [[ "$repo_root" == /* && -d "$repo_root" && "$(pwd -P)" == "$(cd -P -- "$repo_root" && pwd -P)" ]] || invalid 'module evaluation live repository receipt is malformed'
    [[ "$manifest_copy" == /* && -f "$manifest_copy" && ! -L "$manifest_copy" ]] || invalid 'module evaluation manifest-copy receipt is malformed'
    [[ "$(stat -c '%a' -- "$manifest_copy")" == 400 ]] || invalid 'module evaluation manifest copy is not read-only'
    [[ "$(sha256sum "$manifest_copy" | awk '{print $1}')" == "$assertion_digest" ]] || invalid 'module evaluation manifest copy differs from the active digest'

    case "$phase" in
      s2-skeleton) projection='s2_skeleton_red' ;;
      s4-green) projection='s4_green' ;;
      s5-final) projection='s5_final_green' ;;
    esac

    evaluator="${repo_root}/tests/nix/module-eval.nix"
    [[ -f "$evaluator" && ! -L "$evaluator" ]] || invalid 'tests/nix/module-eval.nix is missing or linked'
    evaluator_output="$(nix eval --impure --json --no-write-lock-file --file "$evaluator" --argstr phase "$phase" --argstr repoRoot "$repo_root")" || exit "$?"
    expected="$(jq -ce --arg projection "$projection" '.module_eval[$projection]' "$manifest_copy")" || invalid 'module evaluation projection is missing from the active manifest'

    jq -e --argjson expected "$expected" '
      (keys == ["results"])
      and (.results | type == "array" and length == 18)
      and ([.results[].name] | length == (unique | length))
      and (all(.results[];
        (keys == ["detail", "name", "status"])
        and (.name | type == "string" and length > 0)
        and (.detail | type == "string")
        and (.status == "PASS" or .status == "FAIL" or .status == "SKIP")
      ))
      and ([.results[] | select(.status == "PASS") | .name] == $expected.pass)
      and ([.results[] | select(.status == "FAIL") | .name] == $expected.fail)
      and ([.results[] | select(.status == "SKIP") | "\(.name): \(.detail)"] == $expected.skip)
    ' <<<"$evaluator_output" >/dev/null || invalid 'module evaluation result differs from the frozen phase oracle'

    jq -r '.results[] | if .detail == "" then "\(.status): \(.name)" else "\(.status): \(.name) — \(.detail)" end' <<<"$evaluator_output"
    pass_count="$(jq '[.results[] | select(.status == "PASS")] | length' <<<"$evaluator_output")"
    fail_count="$(jq '[.results[] | select(.status == "FAIL")] | length' <<<"$evaluator_output")"
    skip_count="$(jq '[.results[] | select(.status == "SKIP")] | length' <<<"$evaluator_output")"
    printf 'SUMMARY: %s PASS / %s FAIL / %s SKIP\n' "$pass_count" "$fail_count" "$skip_count"

    if [[ "$phase" == s2-skeleton ]]; then
      printf '%s\n' 'DISPOSITION: BASELINE_RED (twelve named failures)'
      exit 1
    fi

    printf '%s\n' 'DISPOSITION: GREEN (zero named failures)'
    if [[ "$phase" == s5-final ]]; then
      helper="${TASK13_R21_FINAL_TREE_HELPER_COPY:-}"
      helper_sha="${TASK13_R21_FINAL_TREE_HELPER_SHA256:-}"
      verify_helper_copy "$helper" "$helper_sha" 'final-tree helper copy'
      "$helper" prepare-snapshot "$assertion_digest"
    fi

[private]
linux-builder-r21-exec gpe_map_digest replay_mode='': require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    generation_id='task13-r21-g0-f9-frozen-coverage-entrypoint-20260907'
    [[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation_id" ]] || { printf '%s\n' 'error: x86_64-linux descriptor requires guarded R21 generation selection' >&2; exit 2; }
    [[ "${TASK13_ASSERTION_DIGEST:-}" =~ ^[0-9a-f]{64}$ && "$gpe_map_digest" =~ ^[0-9a-f]{64}$ ]] || { printf '%s\n' 'error: x86_64-linux descriptor digest arguments are malformed' >&2; exit 2; }
    [[ -z "$replay_mode" || "$replay_mode" == fv07-replay ]] || { printf '%s\n' 'error: x86_64-linux descriptor replay mode is invalid' >&2; exit 2; }
    helper="${TASK13_R21_FINAL_TREE_HELPER_COPY:-}"
    helper_sha="${TASK13_R21_FINAL_TREE_HELPER_SHA256:-}"
    [[ "$helper" == /* && -f "$helper" && ! -L "$helper" && "$(stat -c '%a' -- "$helper")" == 400 ]] || { printf '%s\n' 'error: x86_64-linux descriptor helper receipt is malformed' >&2; exit 2; }
    [[ "$(sha256sum "$helper" | awk '{print $1}')" == "$helper_sha" && "$helper_sha" =~ ^[0-9a-f]{64}$ ]] || { printf '%s\n' 'error: x86_64-linux descriptor helper content changed' >&2; exit 2; }
    export TASK13_R21_GPE_MAP_SHA256="$gpe_map_digest"
    exec "$helper" run-descriptor "${TASK13_ASSERTION_DIGEST}" x86_64-linux "$gpe_map_digest" "${replay_mode:-first}"

[private]
aarch64-linux-builder-r21-exec gpe_map_digest replay_mode='': require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    generation_id='task13-r21-g0-f9-frozen-coverage-entrypoint-20260907'
    [[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation_id" ]] || { printf '%s\n' 'error: aarch64-linux descriptor requires guarded R21 generation selection' >&2; exit 2; }
    [[ "${TASK13_ASSERTION_DIGEST:-}" =~ ^[0-9a-f]{64}$ && "$gpe_map_digest" =~ ^[0-9a-f]{64}$ ]] || { printf '%s\n' 'error: aarch64-linux descriptor digest arguments are malformed' >&2; exit 2; }
    [[ -z "$replay_mode" || "$replay_mode" == fv07-replay ]] || { printf '%s\n' 'error: aarch64-linux descriptor replay mode is invalid' >&2; exit 2; }
    helper="${TASK13_R21_FINAL_TREE_HELPER_COPY:-}"
    helper_sha="${TASK13_R21_FINAL_TREE_HELPER_SHA256:-}"
    [[ "$helper" == /* && -f "$helper" && ! -L "$helper" && "$(stat -c '%a' -- "$helper")" == 400 ]] || { printf '%s\n' 'error: aarch64-linux descriptor helper receipt is malformed' >&2; exit 2; }
    [[ "$(sha256sum "$helper" | awk '{print $1}')" == "$helper_sha" && "$helper_sha" =~ ^[0-9a-f]{64}$ ]] || { printf '%s\n' 'error: aarch64-linux descriptor helper content changed' >&2; exit 2; }
    export TASK13_R21_GPE_MAP_SHA256="$gpe_map_digest"
    exec "$helper" run-descriptor "${TASK13_ASSERTION_DIGEST}" aarch64-linux "$gpe_map_digest" "${replay_mode:-first}"

[private]
cross-system-verify-r21-exec gpe_map_digest replay_mode='': require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    generation_id='task13-r21-g0-f9-frozen-coverage-entrypoint-20260907'
    [[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation_id" ]] || { printf '%s\n' 'error: aarch64-darwin descriptor requires guarded R21 generation selection' >&2; exit 2; }
    [[ "${TASK13_ASSERTION_DIGEST:-}" =~ ^[0-9a-f]{64}$ && "$gpe_map_digest" =~ ^[0-9a-f]{64}$ ]] || { printf '%s\n' 'error: aarch64-darwin descriptor digest arguments are malformed' >&2; exit 2; }
    [[ -z "$replay_mode" || "$replay_mode" == fv07-replay ]] || { printf '%s\n' 'error: aarch64-darwin descriptor replay mode is invalid' >&2; exit 2; }
    helper="${TASK13_R21_FINAL_TREE_HELPER_COPY:-}"
    helper_sha="${TASK13_R21_FINAL_TREE_HELPER_SHA256:-}"
    [[ "$helper" == /* && -f "$helper" && ! -L "$helper" && "$(stat -c '%a' -- "$helper")" == 400 ]] || { printf '%s\n' 'error: aarch64-darwin descriptor helper receipt is malformed' >&2; exit 2; }
    [[ "$(sha256sum "$helper" | awk '{print $1}')" == "$helper_sha" && "$helper_sha" =~ ^[0-9a-f]{64}$ ]] || { printf '%s\n' 'error: aarch64-darwin descriptor helper content changed' >&2; exit 2; }
    export TASK13_R21_GPE_MAP_SHA256="$gpe_map_digest"
    exec "$helper" run-descriptor "${TASK13_ASSERTION_DIGEST}" aarch64-darwin "$gpe_map_digest" "${replay_mode:-first}"

[private]
final-verify-r21-exec fv_id gpe_map_digest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    generation_id='task13-r21-g0-f9-frozen-coverage-entrypoint-20260907'
    [[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation_id" ]] || { printf '%s\n' 'error: final verification requires guarded R21 generation selection' >&2; exit 2; }
    [[ "${TASK13_ASSERTION_DIGEST:-}" =~ ^[0-9a-f]{64}$ && "$gpe_map_digest" =~ ^[0-9a-f]{64}$ ]] || { printf '%s\n' 'error: final verification digest arguments are malformed' >&2; exit 2; }
    case "$fv_id" in
      fv-01-pf1-fingerprint|fv-02-operation-event-matrix|fv-03-consolidated-whole-tree|fv-04-cross-feature-recovery-privacy-lease-voice|fv-05-migration-adr-chain|fv-06-c2-provenance|fv-07-supported-system-builder-matrix|fv-08-dependency-license-log-gitleaks) ;;
      *) printf '%s\n' 'error: final verification selector is not allowlisted' >&2; exit 2 ;;
    esac
    final_helper="${TASK13_R21_FINAL_VERIFY_HELPER_COPY:-}"
    final_helper_sha="${TASK13_R21_FINAL_VERIFY_HELPER_SHA256:-}"
    tree_helper="${TASK13_R21_FINAL_TREE_HELPER_COPY:-}"
    tree_helper_sha="${TASK13_R21_FINAL_TREE_HELPER_SHA256:-}"
    for helper_tuple in "final:$final_helper:$final_helper_sha" "tree:$tree_helper:$tree_helper_sha"; do
      label="${helper_tuple%%:*}"
      remainder="${helper_tuple#*:}"
      helper_path="${remainder%:*}"
      helper_sha="${remainder##*:}"
      [[ "$helper_path" == /* && -f "$helper_path" && ! -L "$helper_path" && "$(stat -c '%a' -- "$helper_path")" == 400 ]] || { printf 'error: %s helper receipt is malformed\n' "$label" >&2; exit 2; }
      [[ "$helper_sha" =~ ^[0-9a-f]{64}$ && "$(sha256sum "$helper_path" | awk '{print $1}')" == "$helper_sha" ]] || { printf 'error: %s helper content changed\n' "$label" >&2; exit 2; }
    done
    export TASK13_R21_GPE_MAP_SHA256="$gpe_map_digest"
    exec "$final_helper" "${TASK13_ASSERTION_DIGEST}" "$fv_id"
