#!/usr/bin/env bash
set -euo pipefail

# This is a hostile-mutation fixture, never a production executor.  It makes a
# disposable full repository replica, substitutes only `just`, and proves that
# the production R25 guard stops before that nested command on every attack.
readonly generation_id='task13-r25-g0-f13-trust-root-cross-binding-20260907'
readonly manifest_relative='tests/nix/task-13-assertion-manifest-r25-g0-f13.json'
readonly lock_relative='tests/nix/task-13-assertion-manifest-r25-g0-f13.sha256'
readonly record_relative='.agents/results/task-13-s2-r25-g0-f13-manifest-digest-20260907.txt'
readonly shape_relative='scripts/tasks/task-13/r25-shape.just'
readonly entrypoint_relative='scripts/tasks/task-13/strict-entrypoint-r25.sh'
readonly script_directory="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly script_path="${script_directory}/$(basename -- "${BASH_SOURCE[0]}")"
readonly repo_root="$(cd -P -- "$script_directory/../.." && pwd -P)"
readonly temp_root="$(cd -P -- "${TMPDIR:-/tmp}" && pwd -P)"
declare -a disposable_roots=()
active_child_pid=''

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }
sha256_file() { sha256sum "$1" | awk '{print $1}'; }
cleanup() {
  if [[ -n "$active_child_pid" ]]; then kill "$active_child_pid" 2>/dev/null || true; wait "$active_child_pid" 2>/dev/null || true; fi
  local root
  for root in "${disposable_roots[@]}"; do
    case "$root" in "$temp_root"/jamye-task13-r25-toctou.*) rm -rf -- "$root";; *) printf '%s\n' 'error: refusing unsafe fixture cleanup path' >&2;; esac
  done
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

[[ "$(pwd -P)" == "$repo_root" ]] || fail 'fixture must start at repository root'
[[ -n "${IN_NIX_SHELL:-}" ]] || fail 'fixture requires already-active repository Nix devShell'
[[ -f "$manifest_relative" && ! -L "$manifest_relative" ]] || fail 'R25 manifest is missing or linked'
cmp -s <(jq -cS '.' "$manifest_relative") "$manifest_relative" || fail 'R25 manifest is not canonical JSON plus LF'
readonly manifest_digest="$(sha256_file "$manifest_relative")"
cmp -s <(printf '%s\n' "$manifest_digest") "$lock_relative" || fail 'R25 manifest lock mismatch'
cmp -s <(printf '%s\n' "$manifest_digest") "$record_relative" || fail 'R25 manifest record mismatch'
readonly expected_fixture_sha="$(jq -er --arg path 'tests/nix/coverage-executor-toctou-r25.sh' '[.command_source_integrity.sources[] | select(.path == $path)] | if length == 1 then .[0].sha256 else error("fixture row") end' "$manifest_relative")"
[[ "$(sha256_file "$script_path")" == "$expected_fixture_sha" ]] || fail 'fixture bytes differ from R25 manifest'
readonly real_just="$(command -v just)"
[[ "$real_just" == /* && -x "$real_just" ]] || fail 'Just executable is unavailable'

make_replica() {
  local root="$1" relative source_dir physical_dir source destination
  local -a required=(
    "$manifest_relative"
    "$lock_relative"
    "$record_relative"
    "$entrypoint_relative"
    'scripts/tasks/task-13/guarded-just-r25.sh'
    "$shape_relative"
  )
  mkdir -p -- "$root/repo"
  # The isolated replica deliberately contains only the strict bootstrap and
  # the live coverage selection.  It never copies ignored build output, VCS
  # metadata, or unrelated agent state.
  for relative in "${required[@]}"; do
    [[ "$relative" =~ ^[A-Za-z0-9._/-]+$ && "$relative" != /* && "$relative" != *'..'* ]] || fail "noncanonical replica path: ${relative}"
    source_dir="$repo_root/$(dirname -- "$relative")"
    [[ -d "$source_dir" ]] || fail "replica source directory missing: ${relative}"
    physical_dir="$(cd -P -- "$source_dir" && pwd -P)" || fail "replica source directory is not physical: ${relative}"
    source="$physical_dir/$(basename -- "$relative")"
    [[ "$source" == "$repo_root/$relative" && -f "$source" && ! -L "$source" ]] || fail "replica source is not a regular non-symlink: ${relative}"
    destination="$root/repo/$relative"
    mkdir -p -- "$(dirname -- "$destination")"
    cp -- "$source" "$destination"
    [[ -f "$destination" && ! -L "$destination" && "$(sha256_file "$destination")" == "$(sha256_file "$source")" ]] || fail "replica copy differs: ${relative}"
  done
}
replace_file() {
  local target="$1" saved="${1}.before-replacement"
  mv -- "$target" "$saved"
  cp -- "$saved" "$target"
}
mutate_content() { printf '%s\n' '# R25 disposable TOCTOU mutation' >>"$1"; }

run_case() {
  local mode="$1" root sandbox fake_bin entry_copy entry_sha sentinel output ready release
  local ready_fd release_fd child_status ready_message source_path executable_path
  root="$(mktemp -d "$temp_root/jamye-task13-r25-toctou.XXXXXX")" || fail 'cannot create disposable root'
  root="$(cd -P -- "$root" && pwd -P)"; disposable_roots+=("$root")
  make_replica "$root"; sandbox="$root/repo"
  fake_bin="$root/fake-bin"; mkdir -p -- "$fake_bin"
  printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' ': > "${TASK13_R25_NESTED_SENTINEL:?}"' 'exit 0' >"$fake_bin/just"
  chmod 0500 -- "$fake_bin/just"
  entry_copy="$(mktemp "$root/strict-entrypoint.XXXXXX")" || fail 'cannot create external entrypoint copy'
  cp -- "$sandbox/$entrypoint_relative" "$entry_copy"; chmod 0400 -- "$entry_copy"
  entry_sha="$(sha256_file "$entry_copy")"
  sentinel="$root/nested-dispatch-sentinel"; output="$root/output"
  ready="$root/ready.fifo"; release="$root/release.fifo"; mkfifo -- "$ready" "$release"
  exec {ready_fd}<>"$ready"; exec {release_fd}<>"$release"

  if [[ "$mode" == source-precopy-content ]]; then mutate_content "$sandbox/$shape_relative"; fi
  (
    cd "$sandbox"
    if [[ "$mode" == control || "$mode" == source-precopy-content ]]; then
      PATH="$fake_bin:$PATH" TASK13_R25_NESTED_SENTINEL="$sentinel" TASK13_R25_ENTRYPOINT_SHA256="$entry_sha" \
        bash "$entry_copy" "$manifest_digest" pair "$shape_relative" coverage-r25-exec
    else
      PATH="$fake_bin:$PATH" TASK13_R25_NESTED_SENTINEL="$sentinel" TASK13_R25_ENTRYPOINT_SHA256="$entry_sha" \
        TASK13_R25_FIXTURE_MODE="$mode" TASK13_R25_FIXTURE_REPO_ROOT="$sandbox" \
        TASK13_R25_FIXTURE_READY_FD="$ready_fd" TASK13_R25_FIXTURE_RELEASE_FD="$release_fd" \
        bash "$entry_copy" "$manifest_digest" pair "$shape_relative" coverage-r25-exec
    fi
  ) >"$output" 2>&1 &
  active_child_pid=$!

  if [[ "$mode" != control && "$mode" != source-precopy-content ]]; then
    IFS= read -r -t 15 ready_message <&"$ready_fd" || fail "fixture barrier was not reached for ${mode}"
    IFS=: read -r _ reported_mode source_path executable_path <<<"$ready_message"
    [[ "$reported_mode" == "$mode" && "$source_path" == "$sandbox"/* && "$executable_path" == /* ]] || fail "fixture ready receipt is invalid for ${mode}"
    case "$mode" in
      source-postcopy-content) mutate_content "$source_path" ;;
      source-postcopy-replacement) replace_file "$source_path" ;;
      copy-content) mutate_content "$executable_path" ;;
      copy-replacement) replace_file "$executable_path" ;;
    esac
    printf 'release:%s\n' "$mode" >&"$release_fd"
  fi
  set +e; wait "$active_child_pid"; child_status=$?; set -e; active_child_pid=''
  exec {ready_fd}>&-; exec {release_fd}>&-
  if [[ "$mode" == control ]]; then
    [[ "$child_status" == 0 ]] || fail 'control did not exit 0'
    [[ -f "$sentinel" ]] || fail 'control did not reach exactly one nested Just dispatch'
  else
    [[ "$child_status" == 2 ]] || fail "${mode} did not exit 2"
    [[ ! -e "$sentinel" ]] || fail "${mode} reached nested Just dispatch"
  fi
  # All writes are constrained to this disposable replica and fixture root;
  # no original repository path is ever used as a write target.
  [[ ! -e "$repo_root/.task13-r25-fixture-write" ]] || fail 'fixture wrote to persistent repository'
  printf 'PASS: R25 TOCTOU %s exit=%s nested=%s persistent_writes=0\n' "$mode" "$child_status" "$([[ -e "$sentinel" ]] && printf 1 || printf 0)"
}

for mode in control source-precopy-content source-postcopy-content source-postcopy-replacement copy-content copy-replacement; do
  run_case "$mode"
done
printf '%s\n' 'DISPOSITION: GREEN (six isolated R25 TOCTOU modes)'
