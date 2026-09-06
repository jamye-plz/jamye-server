#!/usr/bin/env bash
set -euo pipefail

# Shape-only contract: probes consume attribute names and never evaluate or
# serialize nixosModules.default, which is function-valued.
readonly manifest='tests/nix/task-13-assertion-manifest.json'
readonly expected_top_level='checks,devShells,nixosModules,packages'
readonly expected_packages='api,worker'
readonly expected_devshells='default'
readonly invalid_evidence_exit=70

invalid() {
  printf 'INVALID: %s\n' "$1" >&2
  exit "$invalid_evidence_exit"
}

pass() {
  printf 'PASS: %s%s\n' "$1" "${2:+ — $2}"
}

skip() {
  printf 'SKIP: %s%s\n' "$1" "${2:+ — $2}"
}

declare -a failure_names=()
fail() {
  failure_names+=("$1")
  printf 'FAIL: %s%s\n' "$1" "${2:+ — $2}" >&2
}

joined_names() {
  local output_suffix="$1"
  nix eval --impure --json --no-write-lock-file --expr \
    "$(flake_outputs_expression "builtins.attrNames outputs${output_suffix}")" \
    | jq -r '.[]' \
    | LC_ALL=C sort \
    | paste -sd, -
}

module_default_is_present() {
  nix eval --impure --json --no-write-lock-file --expr \
    "$(flake_outputs_expression 'outputs.nixosModules ? default')" \
    | jq -r 'if type == "boolean" then . else error("nonboolean module presence") end'
}

flake_outputs_expression() {
  local body="$1"
  # getFlake returns a locked flake object whose output attributes are mixed
  # with metadata. Import the filtered snapshot's definition and call its
  # declared outputs function so attrNames targets only flake-defined outputs.
  printf '%s\n' "let
    snapshot = builtins.toPath \"${source_snapshot}\";
    loaded = builtins.getFlake \"path:${source_snapshot}\";
    definition = import (snapshot + \"/flake.nix\");
    outputs = definition.outputs (loaded.inputs // { self = loaded; });
  in ${body}"
}

same_failure_inventory() {
  local index
  ((${#failure_names[@]} == ${#baseline_failures[@]})) || return 1
  for index in "${!baseline_failures[@]}"; do
    [[ "${failure_names[$index]}" == "${baseline_failures[$index]}" ]] || return 1
  done
}

diff_detail() {
  local observed="$1"
  local expected="$2"
  local missing=''
  local extra=''
  local entry
  for entry in ${expected//,/ }; do
    [[ ",$observed," == *",$entry,"* ]] || missing="${missing}${missing:+,}$entry"
  done
  for entry in ${observed//,/ }; do
    [[ ",$expected," == *",$entry,"* ]] || extra="${extra}${extra:+,}$entry"
  done
  if [[ -n "$missing" && -n "$extra" ]]; then
    printf 'missing %s; extra %s' "$missing" "$extra"
  elif [[ -n "$missing" ]]; then
    printf 'missing %s' "$missing"
  else
    printf 'extra %s' "$extra"
  fi
}

[[ -f "$manifest" ]] || invalid 'Task-13 assertion manifest missing'
jq -e '
  (.flake_shape.expected_systems | type == "array" and length == 2)
  and (.flake_shape.checks_key_set | type == "array" and length == 8)
  and (.flake_shape.baseline_red_failures | type == "array" and length == 3)
  and (.flake_shape.per_system_family_system_key_sets.families == ["packages", "devShells", "checks"])
' "$manifest" >/dev/null || invalid 'Task-13 assertion manifest schema is incompatible'

mapfile -t expected_system_array < <(jq -er '.flake_shape.expected_systems[]' "$manifest")
mapfile -t expected_family_array < <(jq -er '.flake_shape.per_system_family_system_key_sets.families[]' "$manifest")
mapfile -t baseline_failures < <(jq -er '.flake_shape.baseline_red_failures[]' "$manifest")
expected_systems="$(printf '%s\n' "${expected_system_array[@]}" | LC_ALL=C sort | paste -sd, -)"
expected_checks="$(jq -er '.flake_shape.checks_key_set | sort | join(",")' "$manifest")"
[[ "$expected_systems" == 'aarch64-darwin,x86_64-linux' ]] || invalid 'locked manifest expected systems are incompatible'
[[ "$expected_checks" == 'api,architecture,cargo-clippy,cargo-fmt,cargo-test-all-features,cargo-test-default,contract-drift,worker' ]] || invalid 'locked manifest checks inventory is incompatible'
[[ "${baseline_failures[0]}" == 'top-level flake output key-set mismatch (missing nixosModules)' && "${baseline_failures[1]}" == 'packages.<system> key-set mismatch (extra default)' && "${baseline_failures[2]}" == 'checks.<system> key-set mismatch (missing contract-drift)' ]] || invalid 'locked manifest baseline failure inventory is incompatible'
# R11 exposes no structured top-level/packages/devShell child inventories. These
# three fixed expectations are the remaining approved t13-s1 literals; all
# structured systems, checks, family, and failure identity comes from manifest.
[[ "$expected_top_level" == 'checks,devShells,nixosModules,packages' && "$expected_packages" == 'api,worker' && "$expected_devshells" == 'default' ]] || invalid 'fixed t13-s1 inventory conformance failure'

repo_root="$(pwd -P)"
[[ -f "$repo_root/flake.nix" && -f "$repo_root/rust-toolchain.toml" ]] || invalid 'repository source root is incomplete'
if ! source_snapshot="$(bash scripts/tasks/task-13/flake-source-snapshot.sh)"; then
  invalid 'filtered source snapshot creation failed'
fi
[[ "$source_snapshot" == /nix/store/* && -f "$source_snapshot/flake.nix" && -f "$source_snapshot/rust-toolchain.toml" ]] || invalid 'filtered source snapshot is incomplete'
readonly source_snapshot
readonly snapshot_flake="$source_snapshot/flake.nix"

if ! top_level="$(joined_names '')"; then
  invalid 'top-level flake output probe failed'
fi
if [[ "$top_level" == "$expected_top_level" ]]; then
  pass 'top-level flake output key-set exact' "$top_level"
elif [[ "$top_level" == 'checks,devShells,packages' ]]; then
  fail "${baseline_failures[0]}"
else
  fail 'top-level flake output key-set mismatch' "$(diff_detail "$top_level" "$expected_top_level")"
fi

family_system_details=()
family_system_ok=true
for family in "${expected_family_array[@]}"; do
  if ! observed="$(joined_names ".${family}")"; then
    invalid "${family} system-key probe failed"
  fi
  family_system_details+=("${family}=${observed}")
  [[ "$observed" == "$expected_systems" ]] || family_system_ok=false
done
if "$family_system_ok"; then
  pass 'per-system output family system key-sets exact' "${family_system_details[*]}"
else
  fail 'per-system output family system key-sets exact' "${family_system_details[*]}"
fi

package_mismatches=()
devshell_mismatches=()
checks_mismatches=()
for system in "${expected_system_array[@]}"; do
  if ! package_keys="$(joined_names ".packages.${system}")"; then
    invalid "packages.${system} child-key probe failed"
  fi
  [[ "$package_keys" == "$expected_packages" ]] || package_mismatches+=("${system}: $(diff_detail "$package_keys" "$expected_packages")")

  if ! devshell_keys="$(joined_names ".devShells.${system}")"; then
    invalid "devShells.${system} child-key probe failed"
  fi
  if [[ "$devshell_keys" == "$expected_devshells" ]]; then
    pass "devShells.${system} key-set exact (default only)" "$devshell_keys"
  else
    devshell_mismatches+=("${system}: $(diff_detail "$devshell_keys" "$expected_devshells")")
  fi

  if ! checks_keys="$(joined_names ".checks.${system}")"; then
    invalid "checks.${system} child-key probe failed"
  fi
  [[ "$checks_keys" == "$expected_checks" ]] || checks_mismatches+=("${system}: $(diff_detail "$checks_keys" "$expected_checks")")
done

if ((${#package_mismatches[@]} == 0)); then
  pass 'packages.<system> key-set exact (api,worker)'
elif [[ "${package_mismatches[*]}" == 'aarch64-darwin: extra default x86_64-linux: extra default' ]]; then
  fail "${baseline_failures[1]}"
else
  fail 'packages.<system> key-set mismatch' "${package_mismatches[*]}"
fi
if ((${#devshell_mismatches[@]} != 0)); then
  fail 'devShells.<system> key-set mismatch' "${devshell_mismatches[*]}"
fi
if ((${#checks_mismatches[@]} == 0)); then
  pass 'checks.<system> key-set exact'
elif [[ "${checks_mismatches[*]}" == 'aarch64-darwin: missing contract-drift x86_64-linux: missing contract-drift' ]]; then
  fail "${baseline_failures[2]}"
else
  fail 'checks.<system> key-set mismatch' "${checks_mismatches[*]}"
fi

module_skip=false
if [[ ",$top_level," == *',nixosModules,'* ]]; then
  if ! module_keys="$(joined_names '.nixosModules')"; then
    invalid 'nixosModules child-key probe failed'
  fi
  if [[ "$module_keys" == 'default' ]]; then
    pass 'nixosModules child key-set exact (default only)' "$module_keys"
  else
    fail 'nixosModules child key-set mismatch' "$(diff_detail "$module_keys" 'default')"
  fi
  if ! module_default_present="$(module_default_is_present)"; then
    invalid 'nixosModules.default presence probe failed'
  fi
  if [[ "$module_default_present" == true ]]; then
    pass 'nixosModules.default presence exact' 'true (function not evaluated)'
  else
    fail 'nixosModules.default presence exact' 'default missing'
  fi
else
  module_skip=true
  skip 'nixosModules child key-set exact (default only)' 'blocked/skipped: nixosModules parent absent'
fi

supported_systems="$(sed -n '/supportedSystems = \[/,/\];/p' "$snapshot_flake" | sed -n 's/^[[:space:]]*"\([^"]*\)".*/\1/p' | paste -sd, -)"
if [[ "$supported_systems" == "$expected_systems" ]]; then
  pass 'flake.nix supportedSystems exact ordered list' "$supported_systems"
else
  fail 'flake.nix supportedSystems exact ordered list' "$(diff_detail "$supported_systems" "$expected_systems")"
fi

snapshot_nix_inputs=("$snapshot_flake")
if [[ -d "$source_snapshot/nix" ]]; then
  snapshot_nix_inputs+=("$source_snapshot/nix")
fi
toolchain_calls="$(rg -n --glob '*.nix' 'fromToolchainFile' "${snapshot_nix_inputs[@]}" 2>/dev/null || true)"
toolchain_call_count="$(printf '%s\n' "$toolchain_calls" | sed '/^$/d' | wc -l | tr -d '[:space:]')"
toolchain_declarations="$(rg -n --glob '*.nix' '^[[:space:]]*(channel|profile|components|targets)[[:space:]]*=' "${snapshot_nix_inputs[@]}" 2>/dev/null || true)"
if [[ "$toolchain_call_count" == 1 && -z "$toolchain_declarations" && "$toolchain_calls" == *'fromToolchainFile'* && "$(rg -c '^[[:space:]]*file = \./rust-toolchain\.toml;' "$snapshot_flake")" == 1 ]]; then
  pass 'sole fenix.fromToolchainFile rust-toolchain.toml declaration invariant'
else
  fail 'sole fenix.fromToolchainFile rust-toolchain.toml declaration invariant' 'duplicate or missing declaration'
fi

if ((${#failure_names[@]} == 0)); then
  printf 'DISPOSITION: GREEN (zero named failures)\n'
  exit 0
fi
if same_failure_inventory && [[ "$family_system_ok" == true && "$module_skip" == true ]]; then
  printf 'DISPOSITION: BASELINE_RED (three named failures)\n'
  exit 1
fi
invalid "unexpected assertion outcome: ${failure_names[*]}"
