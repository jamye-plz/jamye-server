#!/usr/bin/env bash
set -euo pipefail

readonly generation_id='task13-r25-g0-f13-trust-root-cross-binding-20260907'
readonly manifest='tests/nix/task-13-assertion-manifest-r25-g0-f13.json'
readonly expected_top_level='checks,devShells,nixosModules,packages'
readonly expected_systems='aarch64-darwin,aarch64-linux,x86_64-linux'
readonly expected_packages='api,worker'
readonly expected_devshells='default'
readonly expected_checks='api,architecture,cargo-clippy,cargo-fmt,cargo-test-all-features,cargo-test-default,contract-drift,worker'
readonly invalid_evidence_exit=70

invalid() { printf 'INVALID: %s\n' "$1" >&2; exit "$invalid_evidence_exit"; }
pass() { printf 'PASS: %s%s\n' "$1" "${2:+ — $2}"; }
fail() { failures+=("$1"); printf 'FAIL: %s%s\n' "$1" "${2:+ — $2}" >&2; }

contains_csv_item() { [[ ",$1," == *",$2,"* ]]; }

diff_detail() {
  local observed="$1" expected="$2" missing='' extra='' item
  for item in ${expected//,/ }; do contains_csv_item "$observed" "$item" || missing="${missing}${missing:+,}$item"; done
  for item in ${observed//,/ }; do contains_csv_item "$expected" "$item" || extra="${extra}${extra:+,}$item"; done
  if [[ -n "$missing" && -n "$extra" ]]; then printf 'missing %s; extra %s' "$missing" "$extra"
  elif [[ -n "$missing" ]]; then printf 'missing %s' "$missing"
  elif [[ -n "$extra" ]]; then printf 'extra %s' "$extra"
  else printf 'exact'; fi
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

joined_names() {
  local suffix="$1"
  nix eval --impure --json --no-write-lock-file --expr "$(flake_outputs_expression "builtins.attrNames outputs${suffix}")" \
    | jq -r '.[]' | LC_ALL=C sort | paste -sd, -
}

module_default_is_present() {
  nix eval --impure --json --no-write-lock-file --expr \
    "$(flake_outputs_expression 'outputs.nixosModules ? default')" \
    | jq -r 'if type == "boolean" then . else error("nonboolean module presence") end'
}

declare -a failures=()
readonly source_snapshot="${TASK13_FLAKE_SOURCE_SNAPSHOT:-}"
[[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation_id" ]] || invalid 'guarded active R25 generation receipt missing'
[[ "${TASK13_ASSERTION_DIGEST:-}" =~ ^[0-9a-f]{64}$ ]] || invalid 'guarded active R25 digest receipt missing'
[[ "$source_snapshot" == /nix/store/* && -d "$source_snapshot" ]] || invalid 'guarded filtered source snapshot is not a Nix store directory'
[[ "$(pwd -P)" == "$source_snapshot" ]] || invalid 'shape process is not running from the guarded snapshot'
[[ -f "$manifest" && ! -L "$manifest" ]] || invalid 'active R25 assertion manifest missing or linked'
cmp -s <(jq -cS '.' "$manifest") "$manifest" || invalid 'active R25 assertion manifest bytes are not canonical JSON plus LF'
[[ "$(sha256sum "$manifest" | awk '{print $1}')" == "$TASK13_ASSERTION_DIGEST" ]] || invalid 'snapshot R25 assertion manifest does not match guarded digest'
jq -e --arg generation "$generation_id" '
  .generation_id == $generation
  and (.flake_shape.active_generation == $generation)
  and (.flake_shape.expected_systems == ["aarch64-darwin", "aarch64-linux", "x86_64-linux"])
  and (.flake_shape.checks_key_set | type == "array" and length == 8)
  and (.flake_shape.green_only == true)
  and ((.flake_shape | has("baseline_red_failures")) | not)
  and ((.flake_shape | has("missing_system_red")) | not)
  and (.flake_shape.per_system_family_system_key_sets.families == ["packages", "devShells", "checks"])
' "$manifest" >/dev/null || invalid 'active R25 assertion manifest schema is incompatible'

[[ -f "$source_snapshot/flake.nix" && -f "$source_snapshot/rust-toolchain.toml" ]] || invalid 'filtered source snapshot is incomplete'
readonly snapshot_flake="$source_snapshot/flake.nix"

top_level="$(joined_names '')" || invalid 'top-level flake output probe failed'
if [[ "$top_level" == "$expected_top_level" ]]; then pass 'top-level flake output key-set exact' "$top_level"
else fail 'top-level flake output key-set mismatch' "$(diff_detail "$top_level" "$expected_top_level")"; fi

declare -A observed_family_systems=()
family_detail=()
for family in packages devShells checks; do
  observed="$(joined_names ".${family}")" || invalid "${family} system-key probe failed"
  observed_family_systems["$family"]="$observed"
  family_detail+=("${family}=${observed}")
done
supported_systems="$(sed -n '/supportedSystems = \[/,/\];/p' "$snapshot_flake" | sed -n 's/^[[:space:]]*"\([^"]*\)".*/\1/p' | paste -sd, -)"
matrix_exact=true
for family in packages devShells checks; do [[ "${observed_family_systems[$family]}" == "$expected_systems" ]] || matrix_exact=false; done
[[ "$supported_systems" == "$expected_systems" ]] || matrix_exact=false
if "$matrix_exact"; then pass 'supported-system matrix exact' "${family_detail[*]} supportedSystems=${supported_systems}"
else
  fail 'supported-system matrix mismatch' "packages=$(diff_detail "${observed_family_systems[packages]}" "$expected_systems"); devShells=$(diff_detail "${observed_family_systems[devShells]}" "$expected_systems"); checks=$(diff_detail "${observed_family_systems[checks]}" "$expected_systems"); supportedSystems=$(diff_detail "$supported_systems" "$expected_systems")"
fi

package_mismatches=(); devshell_mismatches=(); checks_mismatches=(); probed_systems=()
for system in aarch64-darwin aarch64-linux x86_64-linux; do
  common=true
  for family in packages devShells checks; do contains_csv_item "${observed_family_systems[$family]}" "$system" || common=false; done
  "$common" || continue
  probed_systems+=("$system")
  package_keys="$(joined_names ".packages.${system}")" || invalid "packages.${system} child-key probe failed"
  [[ "$package_keys" == "$expected_packages" ]] || package_mismatches+=("${system}: $(diff_detail "$package_keys" "$expected_packages")")
  devshell_keys="$(joined_names ".devShells.${system}")" || invalid "devShells.${system} child-key probe failed"
  if [[ "$devshell_keys" == "$expected_devshells" ]]; then pass "devShells.${system} key-set exact (default only)" "$devshell_keys"; else devshell_mismatches+=("${system}: $(diff_detail "$devshell_keys" "$expected_devshells")"); fi
  check_keys="$(joined_names ".checks.${system}")" || invalid "checks.${system} child-key probe failed"
  [[ "$check_keys" == "$expected_checks" ]] || checks_mismatches+=("${system}: $(diff_detail "$check_keys" "$expected_checks")")
done
if ((${#probed_systems[@]} == 0)); then fail 'existing supported-system child inventory missing' 'no common system key was safe to probe'
else pass 'existing supported-system child probe inventory' "$(printf '%s\n' "${probed_systems[@]}" | paste -sd, -)"; fi
if ((${#package_mismatches[@]} == 0)); then pass 'packages.<existing-system> key-set exact (api,worker)'; else fail 'packages.<existing-system> key-set mismatch' "${package_mismatches[*]}"; fi
if ((${#devshell_mismatches[@]} != 0)); then fail 'devShells.<existing-system> key-set mismatch' "${devshell_mismatches[*]}"; fi
if ((${#checks_mismatches[@]} == 0)); then pass 'checks.<existing-system> key-set exact'; else fail 'checks.<existing-system> key-set mismatch' "${checks_mismatches[*]}"; fi

if [[ ",$top_level," == *',nixosModules,'* ]]; then
  module_keys="$(joined_names '.nixosModules')" || invalid 'nixosModules child-key probe failed'
  [[ "$module_keys" == default ]] && pass 'nixosModules child key-set exact (default only)' "$module_keys" || fail 'nixosModules child key-set mismatch' "$(diff_detail "$module_keys" default)"
  [[ "$(module_default_is_present)" == true ]] && pass 'nixosModules.default presence exact' 'true (function not evaluated)' || fail 'nixosModules.default presence exact' 'default missing'
else fail 'nixosModules child key-set exact (default only)' 'nixosModules parent absent'; fi

snapshot_nix_inputs=("$snapshot_flake"); [[ -d "$source_snapshot/nix" ]] && snapshot_nix_inputs+=("$source_snapshot/nix")
toolchain_calls="$(rg -n --glob '*.nix' 'fromToolchainFile' "${snapshot_nix_inputs[@]}" 2>/dev/null || true)"
toolchain_declarations="$(rg -n --glob '*.nix' '^[[:space:]]*(channel|profile|components|targets)[[:space:]]*=' "${snapshot_nix_inputs[@]}" 2>/dev/null || true)"
if [[ "$(printf '%s\n' "$toolchain_calls" | sed '/^$/d' | wc -l | tr -d '[:space:]')" == 1 && -z "$toolchain_declarations" && "$(rg -c '^[[:space:]]*file = \./rust-toolchain\.toml;' "$snapshot_flake")" == 1 ]]; then
  pass 'sole fenix.fromToolchainFile rust-toolchain.toml declaration invariant'
else fail 'sole fenix.fromToolchainFile rust-toolchain.toml declaration invariant' 'duplicate or missing declaration'; fi

if ((${#failures[@]} == 0)); then printf 'DISPOSITION: GREEN (zero named failures)\n'; exit 0; fi
invalid "R25 GREEN-only assertion mismatch: ${failures[*]}"
