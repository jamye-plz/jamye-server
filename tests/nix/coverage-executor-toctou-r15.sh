#!/usr/bin/env bash
set -euo pipefail

readonly generation_id='task13-r15-g0-f3-coverage-authority-20260905'
readonly manifest='tests/nix/task-13-assertion-manifest-r15-g0-f3.json'
readonly lock='tests/nix/task-13-assertion-manifest-r15-g0-f3.sha256'
readonly record='.agents/results/task-13-s2-r15-g0-f3-manifest-digest-20260905.txt'
readonly script_directory="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly script_path="${script_directory}/$(basename -- "${BASH_SOURCE[0]}")"
readonly repo_root="$(cd -P -- "$script_directory/../.." && pwd -P)"
readonly temp_root="$(cd -P -- "${TMPDIR:-/tmp}" && pwd -P)"
declare -a disposable_roots=()
active_child_pid=''

fail() {
  printf 'FAIL: %s\n' "$1" >&2
  exit 1
}

cleanup() {
  local disposable_root
  if [[ -n "$active_child_pid" ]]; then
    kill "$active_child_pid" 2>/dev/null || true
    wait "$active_child_pid" 2>/dev/null || true
    active_child_pid=''
  fi
  for disposable_root in "${disposable_roots[@]}"; do
    case "$disposable_root" in
      "$temp_root"/jamye-task13-r15-toctou.*) rm -rf -- "$disposable_root" ;;
      *) printf 'error: refusing unsafe fixture cleanup path\n' >&2 ;;
    esac
  done
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

[[ "$(pwd -P)" == "$repo_root" ]] || fail 'fixture must start at the repository root'
[[ -n "${IN_NIX_SHELL:-}" ]] || fail 'fixture requires the already-active repository Nix devShell'
[[ -f "$manifest" && ! -L "$manifest" ]] || fail 'R15 manifest is missing or linked'
cmp -s <(jq -cS '.' "$manifest") "$manifest" || fail 'R15 manifest is not canonical JSON plus LF'
manifest_digest="$(sha256sum "$manifest" | awk '{print $1}')"
cmp -s <(printf '%s\n' "$manifest_digest") "$lock" || fail 'R15 manifest lock mismatch'
cmp -s <(printf '%s\n' "$manifest_digest") "$record" || fail 'R15 coordinator record mismatch'
expected_fixture_sha="$(jq -er '.command_source_integrity.post_validation_mutation_fixture.executable_sha256' "$manifest")" || fail 'fixture digest missing from R15 manifest'
[[ "$(sha256sum "$script_path" | awk '{print $1}')" == "$expected_fixture_sha" ]] || fail 'fixture bytes differ from R15 manifest'
readonly manifest_digest expected_fixture_sha

readonly real_just="$(command -v just)"
[[ "$real_just" == /* && -x "$real_just" ]] || fail 'Just executable is unavailable'

copy_fixture_repository() {
  local destination_root="$1"
  local relative_path destination_directory
  local -a required_paths=(
    scripts/tasks/task-13/mod.just
    scripts/tasks/task-13/r15-shape.just
    scripts/tasks/task-13/guarded-just-r15.sh
    scripts/tasks/task-13/flake-source-snapshot-r15.sh
    tests/nix/flake-shape-aarch64-linux-r15.sh
    tests/nix/coverage-red.sh
    tests/nix/task-13-assertion-manifest-r15-g0-f3.json
    tests/nix/task-13-assertion-manifest-r15-g0-f3.sha256
    tests/nix/task-13-r15-authority-plan-20260905.json
    tests/nix/task-13-r15-authority-requirements-20260905.md
    .agents/results/task-4b-redis-publish-completion-g0-f3-r15-20260905.json
    .agents/results/review-task13-g0-f3-r15-token-integrity-r1-20260905.md
    .agents/results/task-13-s2-r15-g0-f3-manifest-digest-20260905.txt
  )
  for relative_path in "${required_paths[@]}"; do
    [[ -f "$repo_root/$relative_path" && ! -L "$repo_root/$relative_path" ]] || fail "fixture source missing or linked: $relative_path"
    destination_directory="$destination_root/$(dirname -- "$relative_path")"
    mkdir -p -- "$destination_directory"
    cp -- "$repo_root/$relative_path" "$destination_directory/$(basename -- "$relative_path")"
  done
}

run_mutation_case() {
  local fixture_mode="$1"
  local disposable_root sandbox fake_bin target saved_target sentinel output_file ready_pipe release_pipe
  local ready_fd release_fd child_pid ready_message child_status
  disposable_root="$(mktemp -d "$temp_root/jamye-task13-r15-toctou.XXXXXX")" || fail 'cannot create disposable fixture root'
  disposable_root="$(cd -P -- "$disposable_root" && pwd -P)"
  disposable_roots+=("$disposable_root")
  sandbox="$disposable_root/repo"
  mkdir -p -- "$sandbox"
  copy_fixture_repository "$sandbox"
  sandbox="$(cd -P -- "$sandbox" && pwd -P)"
  fake_bin="$disposable_root/fake-bin"
  mkdir -p -- "$fake_bin"
  printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' ': > "${TASK13_R15_NESTED_SENTINEL:?}"' 'exit 0' >"$fake_bin/just"
  chmod 0500 -- "$fake_bin/just"
  sentinel="$disposable_root/nested-dispatch-sentinel"
  output_file="$disposable_root/guard-output.txt"
  ready_pipe="$disposable_root/ready.fifo"
  release_pipe="$disposable_root/release.fifo"
  mkfifo -- "$ready_pipe" "$release_pipe"
  exec {ready_fd}<>"$ready_pipe"
  exec {release_fd}<>"$release_pipe"

  (
    cd "$sandbox"
    PATH="$fake_bin:$PATH" \
    TASK13_R15_NESTED_SENTINEL="$sentinel" \
    TASK13_R15_POST_COPY_FIXTURE_MODE="$fixture_mode" \
    TASK13_R15_FIXTURE_REPO_ROOT="$sandbox" \
    TASK13_R15_FIXTURE_READY_FD="$ready_fd" \
    TASK13_R15_FIXTURE_RELEASE_FD="$release_fd" \
      "$real_just" --justfile scripts/tasks/task-13/mod.just coverage-red "$manifest_digest"
  ) >"$output_file" 2>&1 &
  child_pid=$!
  active_child_pid="$child_pid"

  if ! IFS= read -r -t 15 ready_message <&"$ready_fd"; then
    kill "$child_pid" 2>/dev/null || true
    wait "$child_pid" 2>/dev/null || true
    active_child_pid=''
    fail "fixture barrier was not reached for $fixture_mode"
  fi
  [[ "$ready_message" == "ready:$fixture_mode" ]] || fail "fixture ready token mismatch for $fixture_mode"

  target="$sandbox/scripts/tasks/task-13/mod.just"
  case "$fixture_mode" in
    content-mutation)
      printf '%s\n' '# deterministic disposable content mutation' >>"$target"
      ;;
    path-replacement)
      saved_target="$sandbox/scripts/tasks/task-13/mod.just.before-path-replacement"
      mv -- "$target" "$saved_target"
      cp -- "$saved_target" "$target"
      ;;
    *) fail "unknown fixture mode: $fixture_mode" ;;
  esac
  printf 'release:%s\n' "$fixture_mode" >&"$release_fd"

  set +e
  wait "$child_pid"
  child_status=$?
  set -e
  active_child_pid=''
  exec {ready_fd}>&-
  exec {release_fd}>&-

  [[ "$child_status" == 2 ]] || fail "$fixture_mode did not exit 2"
  [[ ! -e "$sentinel" ]] || fail "$fixture_mode reached nested Just dispatch"
  case "$fixture_mode" in
    content-mutation) rg -Fq 'bound source content changed at live-target-final-pre-dispatch' "$output_file" || fail 'content mutation did not use the production validator' ;;
    path-replacement) rg -Fq 'bound source identity changed at live-target-final-pre-dispatch' "$output_file" || fail 'path replacement did not use the production validator' ;;
  esac
  printf 'PASS: R15 coverage executor %s exits 2 with zero nested dispatch\n' "$fixture_mode"
}

run_mutation_case 'content-mutation'
run_mutation_case 'path-replacement'
printf '%s\n' 'DISPOSITION: GREEN (two deterministic production-validator fixtures)'
