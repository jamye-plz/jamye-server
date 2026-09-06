set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set positional-arguments
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter the repository Nix devShell before running task-13' >&2; exit 2; fi

# R13 successor receipt. The mutable outer Justfile transports only the
# literal generation and mode; the frozen R13 guard performs every check.
r13-manifest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    exec bash scripts/tasks/task-13/guarded-just-r13.sh task13-r13-g0-f1-format-drift-20260905 --receipt

# R13 GREEN-only shape adapter. No R13 RED state is accepted or synthesized.
aarch64-linux-shape-r13 recorded_digest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    exec bash scripts/tasks/task-13/guarded-just-r13.sh task13-r13-g0-f1-format-drift-20260905 "$1" scripts/tasks/task-13/r13-shape.just aarch64-linux-shape-r13-immutable

# R13 native flake adapter; unchanged Task-1 semantics execute only from the
# immutable source snapshot selected and verified by the frozen R13 guard.
guarded-flake-local-r13 recorded_digest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    exec bash scripts/tasks/task-13/guarded-just-r13.sh task13-r13-g0-f1-format-drift-20260905 "$1" scripts/tasks/task-1/mod.just flake-local

# R14 authority-snapshot successor receipt. This mutable outer Justfile only
# transports literal argv; the frozen R14 guard owns every trust decision.
r14-manifest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    exec bash scripts/tasks/task-13/guarded-just-r14.sh task13-r14-g0-f2-authority-snapshots-20260905 --receipt

# R14 exact three-system GREEN-only shape card.
aarch64-linux-shape-r14 recorded_digest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    exec bash scripts/tasks/task-13/guarded-just-r14.sh task13-r14-g0-f2-authority-snapshots-20260905 "$1" scripts/tasks/task-13/r14-shape.just aarch64-linux-shape-r14-immutable

# R14 native flake card. The unchanged Task-1 recipe runs only from the one
# filtered source snapshot selected and revalidated by the frozen R14 guard.
guarded-flake-local-r14 recorded_digest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    exec bash scripts/tasks/task-13/guarded-just-r14.sh task13-r14-g0-f2-authority-snapshots-20260905 "$1" scripts/tasks/task-1/mod.just flake-local

# R15 is the sole active authority. This mutable recipe transports literal
# argv only; the frozen guard owns receipt and trust validation.
r15-manifest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    exec bash scripts/tasks/task-13/guarded-just-r15.sh task13-r15-g0-f3-coverage-authority-20260905 --receipt

# R15 exact three-system GREEN-only shape transport.
aarch64-linux-shape-r15 recorded_digest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    exec bash scripts/tasks/task-13/guarded-just-r15.sh task13-r15-g0-f3-coverage-authority-20260905 "$1" scripts/tasks/task-13/r15-shape.just aarch64-linux-shape-r15-immutable

# R15 native flake transport. The unchanged Task-1 recipe runs only from the
# immutable source snapshot selected and revalidated by the frozen guard.
guarded-flake-local-r15 recorded_digest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    exec bash scripts/tasks/task-13/guarded-just-r15.sh task13-r15-g0-f3-coverage-authority-20260905 "$1" scripts/tasks/task-1/mod.just flake-local

# R16 is the sole active authority after its issuance chain completes. This
# mutable outer recipe transports literal argv only.
r16-manifest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    exec bash scripts/tasks/task-13/guarded-just-r16.sh task13-r16-g0-f4-coverage-cli-compat-20260906 --receipt

# R16 exact three-system GREEN-only shape transport.
aarch64-linux-shape-r16 recorded_digest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    exec bash scripts/tasks/task-13/guarded-just-r16.sh task13-r16-g0-f4-coverage-cli-compat-20260906 "$1" scripts/tasks/task-13/r16-shape.just aarch64-linux-shape-r16-immutable

# R16 native flake transport. The frozen guard creates and revalidates the
# filtered immutable source snapshot before selecting the Task-1 recipe.
guarded-flake-local-r16 recorded_digest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    exec bash scripts/tasks/task-13/guarded-just-r16.sh task13-r16-g0-f4-coverage-cli-compat-20260906 "$1" scripts/tasks/task-1/mod.just flake-local

# R16 corrected coverage transport. R15 coverage surfaces remain historical;
# only coverage-r16-exec is an active nested pair.
coverage-r16 recorded_digest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    exec bash scripts/tasks/task-13/guarded-just-r16.sh task13-r16-g0-f4-coverage-cli-compat-20260906 "$1" scripts/tasks/task-13/mod.just coverage-r16-exec

# R20 is the sole active strict authority after manifest issuance.  This
# mutable outer surface transports literal argv only; the frozen R20 guard
# owns every receipt, schema, source, pair, and digest trust decision.
r20-manifest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    exec bash scripts/tasks/task-13/guarded-just-r20.sh task13-r20-g0-f8-token-review-path-consistency-20260907 --receipt

# R20 exact three-system GREEN-only shape transport.
aarch64-linux-shape-r20 recorded_digest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    exec bash scripts/tasks/task-13/guarded-just-r20.sh task13-r20-g0-f8-token-review-path-consistency-20260907 "$1" scripts/tasks/task-13/r20-shape.just aarch64-linux-shape-r20-immutable

# R20 native flake transport through the immutable filtered source snapshot.
guarded-flake-local-r20 recorded_digest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    exec bash scripts/tasks/task-13/guarded-just-r20.sh task13-r20-g0-f8-token-review-path-consistency-20260907 "$1" scripts/tasks/task-1/mod.just flake-local

# R20 serialized coverage transport.  Only the private R20 executor is an
# allowlisted live-worktree pair.
coverage-r20 recorded_digest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    exec bash scripts/tasks/task-13/guarded-just-r20.sh task13-r20-g0-f8-token-review-path-consistency-20260907 "$1" scripts/tasks/task-13/mod.just coverage-r20-exec

# t13-s3 RED argv transport. The public name is deliberately not an R15
# nested pair; only the distinct private executor is allowlisted.
coverage-red recorded_digest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    exec bash scripts/tasks/task-13/guarded-just-r15.sh task13-r15-g0-f3-coverage-authority-20260905 "$1" scripts/tasks/task-13/mod.just coverage-red-r15-exec

# t13-s3 GREEN argv transport. The R15 guard selects only the distinct private
# executor from its verified read-only adjacent copy.
coverage recorded_digest: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    exec bash scripts/tasks/task-13/guarded-just-r15.sh task13-r15-g0-f3-coverage-authority-20260905 "$1" scripts/tasks/task-13/mod.just coverage-r15-exec

[private]
coverage-red-r15-exec: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    readonly generation_id='task13-r15-g0-f3-coverage-authority-20260905'
    [[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation_id" ]] || { printf '%s\n' 'error: coverage RED requires guarded R15 generation selection' >&2; exit 2; }
    [[ "${TASK13_ASSERTION_DIGEST:-}" =~ ^[0-9a-f]{64}$ ]] || { printf '%s\n' 'error: coverage RED guard digest receipt is missing' >&2; exit 2; }
    [[ "${TASK13_BOUND_JUSTFILE:-}" == /* && -f "$TASK13_BOUND_JUSTFILE" && ! -L "$TASK13_BOUND_JUSTFILE" ]] || { printf '%s\n' 'error: coverage RED bound Justfile copy is missing or invalid' >&2; exit 2; }
    [[ "${TASK13_COVERAGE_RED_ASSERTION:-}" == /* && -f "$TASK13_COVERAGE_RED_ASSERTION" && ! -L "$TASK13_COVERAGE_RED_ASSERTION" ]] || { printf '%s\n' 'error: coverage RED assertion copy is missing or invalid' >&2; exit 2; }
    exec bash -- "$TASK13_COVERAGE_RED_ASSERTION"

[private]
coverage-r15-exec: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail

    readonly generation_id='task13-r15-g0-f3-coverage-authority-20260905'
    readonly expected_digest='e4332daf6237645c04e6b473796aeccd7a44481a6c754f4bd364c6ee73d9bbc1'
    readonly expected_rust_version='1.98.0'
    readonly expected_coverage_version='0.9.0'
    readonly coverage_attribute='pkgs.cargo-llvm-cov'
    readonly nixpkgs_revision='56c02bc00adcf003215cc4bd996d6efaf4cff188'
    readonly coverage_target_directory='target/task-13-llvm-cov'
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
        invalid "${mode} target inventory differs from the frozen R15 oracle"
      fi
    }

    [[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation_id" ]] || invalid 'coverage requires guarded R15 generation selection'
    [[ "${TASK13_ASSERTION_DIGEST:-}" == "$expected_digest" ]] || invalid 'coverage requires the exact active R15 manifest digest'
    verify_bound_justfile_copy

    source scripts/tasks/task-1/_common.sh
    load_local_env

    rustc_path=''
    cargo_path=''
    cargo_llvm_cov_path=''
    resolve_store_command rustc rustc_path
    resolve_store_command cargo cargo_path
    resolve_store_command cargo-llvm-cov cargo_llvm_cov_path

    actual_rust_version="$($rustc_path --version | awk '{print $2}')"
    [[ "$actual_rust_version" == "$expected_rust_version" ]] || invalid 'active Rust release differs from frozen R15 coverage authority'
    coverage_version_output="$("$cargo_llvm_cov_path" llvm-cov --version)"
    [[ "$coverage_version_output" == "cargo-llvm-cov ${expected_coverage_version}" ]] || invalid 'cargo-llvm-cov version differs from frozen R15 coverage authority'

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

    readonly evidence_directory="target/task-13-coverage-evidence/${expected_digest}"
    umask 077
    mkdir -p -- "$evidence_directory"
    printf '%s\n' "$default_metadata" >"$evidence_directory/default-target-metadata.json"
    printf '%s\n' "$fixture_metadata" >"$evidence_directory/fixture-all-features-target-metadata.json"

    if CARGO_NET_OFFLINE=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov --locked --offline --workspace --all-targets --no-clean --no-report >"$evidence_directory/default-mode.log" 2>&1; then
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

    if CARGO_NET_OFFLINE=true JAMYE_ENABLE_DEV_FIXTURES=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov --locked --offline --workspace --all-targets --all-features --no-clean --no-report >"$evidence_directory/fixture-all-features-mode.log" 2>&1; then
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
coverage-r20-exec: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail

    readonly generation_id='task13-r20-g0-f8-token-review-path-consistency-20260907'
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
        invalid "${mode} target inventory differs from the frozen R20 oracle"
      fi
    }

    [[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation_id" ]] || invalid 'coverage requires guarded R20 generation selection'
    [[ "$assertion_digest" =~ ^[0-9a-f]{64}$ ]] || invalid 'coverage requires the exact active R20 manifest digest'
    actual_coverage_stream_sha="$(printf '%s\n' "${coverage_commands[@]}" | sha256sum | awk '{print $1}')"
    [[ "$actual_coverage_stream_sha" == "$expected_coverage_stream_sha" ]] || invalid 'coverage command stream differs from frozen R20 authority'
    verify_bound_justfile_copy

    # Keep the environment loader inside the guard-bound adjacent Justfile
    # copy so the active R20 executor has no mutable auxiliary dependency.
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
    [[ "$actual_rust_version" == "$expected_rust_version" ]] || invalid 'active Rust release differs from frozen R20 coverage authority'
    coverage_version_output="$("$cargo_llvm_cov_path" llvm-cov --version)"
    [[ "$coverage_version_output" == "cargo-llvm-cov ${expected_coverage_version}" ]] || invalid 'cargo-llvm-cov version differs from frozen R20 coverage authority'

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
coverage-r16-exec: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail

    readonly generation_id='task13-r16-g0-f4-coverage-cli-compat-20260906'
    readonly expected_digest='d177dfdbda7f25aac02857e6c73d9fb86c378699d1c94ce1ce648b4df6656e79'
    readonly expected_rust_version='1.98.0'
    readonly expected_coverage_version='0.9.0'
    readonly coverage_attribute='pkgs.cargo-llvm-cov'
    readonly nixpkgs_revision='56c02bc00adcf003215cc4bd996d6efaf4cff188'
    readonly coverage_target_directory='target/task-13-llvm-cov'
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
        invalid "${mode} target inventory differs from the frozen R16 oracle"
      fi
    }

    [[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation_id" ]] || invalid 'coverage requires guarded R16 generation selection'
    [[ "${TASK13_ASSERTION_DIGEST:-}" == "$expected_digest" ]] || invalid 'coverage requires the exact active R16 manifest digest'
    verify_bound_justfile_copy

    # Keep the environment loader inside the guard-bound adjacent Justfile
    # copy so the active R16 executor has no mutable auxiliary dependency.
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
    [[ "$actual_rust_version" == "$expected_rust_version" ]] || invalid 'active Rust release differs from frozen R16 coverage authority'
    coverage_version_output="$("$cargo_llvm_cov_path" llvm-cov --version)"
    [[ "$coverage_version_output" == "cargo-llvm-cov ${expected_coverage_version}" ]] || invalid 'cargo-llvm-cov version differs from frozen R16 coverage authority'

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

    readonly evidence_directory="target/task-13-coverage-evidence/${expected_digest}"
    umask 077
    mkdir -p -- "$evidence_directory"
    printf '%s\n' "$default_metadata" >"$evidence_directory/default-target-metadata.json"
    printf '%s\n' "$fixture_metadata" >"$evidence_directory/fixture-all-features-target-metadata.json"

    if CARGO_NET_OFFLINE=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov --locked --offline --workspace --all-targets --no-report >"$evidence_directory/default-mode.log" 2>&1; then
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

    if CARGO_NET_OFFLINE=true JAMYE_ENABLE_DEV_FIXTURES=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov --locked --offline --workspace --all-targets --all-features --no-report >"$evidence_directory/fixture-all-features-mode.log" 2>&1; then
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
