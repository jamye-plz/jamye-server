#!/usr/bin/env bash
set -euo pipefail

# Active R21 GREEN-only shape contract. Top-level system-key probes are
# non-forcing; any missing or extra system/key is invalid acceptance evidence.
readonly generation_id='task13-r21-g0-f9-frozen-coverage-entrypoint-20260907'
readonly manifest='tests/nix/task-13-assertion-manifest-r21-g0-f9.json'
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
  printf '%s\n' "let
    snapshot = builtins.toPath \"${source_snapshot}\";
    loaded = builtins.getFlake \"path:${source_snapshot}\";
    definition = import (snapshot + \"/flake.nix\");
    outputs = definition.outputs (loaded.inputs // { self = loaded; });
  in ${body}"
}

contains_csv_item() {
  local csv="$1"
  local item="$2"
  [[ ",$csv," == *",$item,"* ]]
}

diff_detail() {
  local observed="$1"
  local expected="$2"
  local missing=''
  local extra=''
  local entry
  for entry in ${expected//,/ }; do
    contains_csv_item "$observed" "$entry" || missing="${missing}${missing:+,}$entry"
  done
  for entry in ${observed//,/ }; do
    contains_csv_item "$expected" "$entry" || extra="${extra}${extra:+,}$entry"
  done
  if [[ -n "$missing" && -n "$extra" ]]; then
    printf 'missing %s; extra %s' "$missing" "$extra"
  elif [[ -n "$missing" ]]; then
    printf 'missing %s' "$missing"
  elif [[ -n "$extra" ]]; then
    printf 'extra %s' "$extra"
  else
    printf 'exact'
  fi
}

declare -a failure_names=()

source_snapshot="${TASK13_FLAKE_SOURCE_SNAPSHOT:-}"
[[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation_id" ]] || invalid 'guarded active R21 generation receipt missing'
[[ "${TASK13_ASSERTION_DIGEST:-}" =~ ^[0-9a-f]{64}$ ]] || invalid 'guarded active R21 digest receipt missing'
[[ "$source_snapshot" == /nix/store/* && -d "$source_snapshot" ]] || invalid 'guarded filtered source snapshot is not a Nix store directory'
[[ "$(pwd -P)" == "$source_snapshot" ]] || invalid 'shape process is not running from the guarded filtered source snapshot'
[[ -f "$manifest" ]] || invalid 'active R21 assertion manifest missing'
cmp -s <(jq -cS '.' "$manifest") "$manifest" || invalid 'active R21 assertion manifest bytes are not canonical JSON plus LF'
[[ "$(sha256sum "$manifest" | awk '{print $1}')" == "$TASK13_ASSERTION_DIGEST" ]] || invalid 'snapshot R21 assertion manifest does not match the guarded digest receipt'
jq -e --arg generation_id "$generation_id" '
  .generation_id == $generation_id
  and (.flake_shape.active_generation == $generation_id)
  and (.flake_shape.expected_systems == ["aarch64-darwin", "aarch64-linux", "x86_64-linux"])
  and (.flake_shape.checks_key_set | type == "array" and length == 8)
  and (.flake_shape.green_only == true)
  and ((.flake_shape | has("baseline_red_failures")) | not)
  and ((.flake_shape | has("missing_system_red")) | not)
  and (.flake_shape.per_system_family_system_key_sets.families == ["packages", "devShells", "checks"])
' "$manifest" >/dev/null || invalid 'active R21 assertion manifest schema is incompatible'

mapfile -t expected_system_array < <(jq -er '.flake_shape.expected_systems[]' "$manifest")
mapfile -t expected_family_array < <(jq -er '.flake_shape.per_system_family_system_key_sets.families[]' "$manifest")
expected_systems="$(printf '%s\n' "${expected_system_array[@]}" | paste -sd, -)"
expected_checks="$(jq -er '.flake_shape.checks_key_set | sort | join(",")' "$manifest")"
[[ "$expected_systems" == 'aarch64-darwin,aarch64-linux,x86_64-linux' ]] || invalid 'active R21 expected systems are incompatible'
[[ "$expected_checks" == 'api,architecture,cargo-clippy,cargo-fmt,cargo-test-all-features,cargo-test-default,contract-drift,worker' ]] || invalid 'active R21 checks inventory is incompatible'

[[ "$source_snapshot" == /nix/store/* && -f "$source_snapshot/flake.nix" && -f "$source_snapshot/rust-toolchain.toml" ]] || invalid 'filtered source snapshot is incomplete'
readonly source_snapshot
readonly snapshot_flake="$source_snapshot/flake.nix"

if ! top_level="$(joined_names '')"; then
  invalid 'top-level flake output probe failed'
fi
if [[ "$top_level" == "$expected_top_level" ]]; then
  pass 'top-level flake output key-set exact' "$top_level"
else
  fail 'top-level flake output key-set mismatch' "$(diff_detail "$top_level" "$expected_top_level")"
fi

declare -A observed_family_systems=()
family_system_details=()
for family in "${expected_family_array[@]}"; do
  if ! observed="$(joined_names ".${family}")"; then
    invalid "${family} system-key probe failed"
  fi
  observed_family_systems["$family"]="$observed"
  family_system_details+=("${family}=${observed}")
done

supported_systems="$(sed -n '/supportedSystems = \[/,/\];/p' "$snapshot_flake" | sed -n 's/^[[:space:]]*"\([^"]*\)".*/\1/p' | paste -sd, -)"
matrix_detail="packages=$(diff_detail "${observed_family_systems[packages]}" "$expected_systems"); devShells=$(diff_detail "${observed_family_systems[devShells]}" "$expected_systems"); checks=$(diff_detail "${observed_family_systems[checks]}" "$expected_systems"); supportedSystems=$(diff_detail "$supported_systems" "$expected_systems")"

matrix_exact=true
for family in "${expected_family_array[@]}"; do
  [[ "${observed_family_systems[$family]}" == "$expected_systems" ]] || matrix_exact=false
done
[[ "$supported_systems" == "$expected_systems" ]] || matrix_exact=false

if "$matrix_exact"; then
  pass 'supported-system matrix exact' "${family_system_details[*]} supportedSystems=${supported_systems}"
else
  fail 'supported-system matrix mismatch' "$matrix_detail"
fi

package_mismatches=()
devshell_mismatches=()
checks_mismatches=()
probed_systems=()
for system in "${expected_system_array[@]}"; do
  present_in_every_family=true
  for family in "${expected_family_array[@]}"; do
    contains_csv_item "${observed_family_systems[$family]}" "$system" || present_in_every_family=false
  done
  if ! "$present_in_every_family"; then
    continue
  fi
  probed_systems+=("$system")

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

if ((${#probed_systems[@]} == 0)); then
  fail 'existing supported-system child inventory missing' 'no common system key was safe to probe'
else
  pass 'existing supported-system child probe inventory' "$(printf '%s\n' "${probed_systems[@]}" | paste -sd, -)"
fi
if ((${#package_mismatches[@]} == 0)); then
  pass 'packages.<existing-system> key-set exact (api,worker)'
else
  fail 'packages.<existing-system> key-set mismatch' "${package_mismatches[*]}"
fi
if ((${#devshell_mismatches[@]} != 0)); then
  fail 'devShells.<existing-system> key-set mismatch' "${devshell_mismatches[*]}"
fi
if ((${#checks_mismatches[@]} == 0)); then
  pass 'checks.<existing-system> key-set exact'
else
  fail 'checks.<existing-system> key-set mismatch' "${checks_mismatches[*]}"
fi

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
  fail 'nixosModules child key-set exact (default only)' 'nixosModules parent absent'
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
invalid "R21 GREEN-only assertion mismatch: ${failure_names[*]}"


