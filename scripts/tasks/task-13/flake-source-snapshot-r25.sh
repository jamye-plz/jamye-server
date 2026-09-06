#!/usr/bin/env bash
set -euo pipefail

if (($# != 0)); then
  printf '%s\n' 'error: flake-source-snapshot-r25.sh accepts no arguments' >&2
  exit 2
fi

readonly helper_directory="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly helper_path="$helper_directory/$(basename -- "${BASH_SOURCE[0]}")"
if [[ -n "${TASK13_SOURCE_ROOT:-}" ]]; then
  [[ "$TASK13_SOURCE_ROOT" == /* && -d "$TASK13_SOURCE_ROOT" ]] || {
    printf '%s\n' 'error: injected Task-13 source root is invalid' >&2
    exit 2
  }
  readonly repo_root="$(cd -P -- "$TASK13_SOURCE_ROOT" && pwd -P)"
else
  [[ -f "$helper_path" && ! -L "$helper_path" ]] || {
    printf '%s\n' 'error: flake-source-snapshot helper path is invalid' >&2
    exit 2
  }
  readonly repo_root="$(cd -P -- "$helper_directory/../../.." && pwd -P)"
fi
[[ -f "$repo_root/flake.nix" && -f "$repo_root/rust-toolchain.toml" ]] || {
  printf '%s\n' 'error: repository source root is incomplete' >&2
  exit 2
}

# This filter deliberately has no Git dependency.  It admits only the source
# surface and the generation-specific records that are already immutable before
# an R25 snapshot is requested; all other agent state stays outside the store.
source_snapshot="$({
  TASK13_SOURCE_ROOT="$repo_root" nix eval --impure --raw --expr '
    let
      root = builtins.toPath (builtins.getEnv "TASK13_SOURCE_ROOT");
      rootString = toString root;
      rootFiles = [
        "AGENTS.md" "CLAUDE.md" "Cargo.lock" "Cargo.toml" "Justfile"
        "README.md" "compose.yaml" "deny.toml" "flake.lock" "flake.nix"
        "rust-toolchain.toml" ".env.example"
      ];
      r25RuntimeAuthorityFiles = [
        ".agents/results/task-13-g0-f13-r25-approval-20260907.json"
        ".agents/results/task-4b-redis-publish-completion-g0-f13-r25-20260907.json"
        ".agents/results/review-task13-g0-f13-r25-token-integrity-r1-20260907.json"
        ".agents/results/review-task13-g0-f13-r25-prelock-source-audit-r1-20260907.json"
        ".agents/results/task-13-s2-r25-g0-f13-manifest-digest-20260907.txt"
      ];
      r25AuthorityDirectories = [ ".agents" ".agents/results" ];
      relativePath = path:
        let value = toString path; prefixLength = (builtins.stringLength rootString) + 1;
        in if value == rootString then "" else builtins.substring prefixLength ((builtins.stringLength value) - prefixLength) value;
      included = path: type:
        let
          relative = relativePath path;
          allowedTree = (builtins.match "^(src|tests|migrations|contracts|scripts|nix|data|production_composition|docs)(/.*)?$" relative) != null;
          blockedNamedSegment = (builtins.match "(^|.*/)(target|result|[.]git|[.]agents)(/.*|$)" relative) != null;
          hiddenSegment = (builtins.match "(^|.*/)[.][^/]+(/.*|$)" relative) != null;
          environmentSegment = (builtins.match "(^|.*/)[.]env[^/]*(/.*|$)" relative) != null;
          docsPlans = (builtins.match "^docs/plans(/.*|$)" relative) != null;
          authorityTraversal = type == "directory" && builtins.elem relative r25AuthorityDirectories;
        in type != "symlink" && (
          relative == "" || builtins.elem relative rootFiles || authorityTraversal
          || builtins.elem relative r25RuntimeAuthorityFiles
          || (allowedTree && !(blockedNamedSegment || hiddenSegment || environmentSegment || docsPlans))
        );
    in toString (builtins.path {
      path = root;
      name = "jamye-server-task13-r25-flake-source";
      filter = included;
    })
  '
} )"

[[ "$source_snapshot" == /nix/store/* && -d "$source_snapshot" ]] || {
  printf '%s\n' 'error: filtered source snapshot is not a Nix store directory' >&2
  exit 2
}
[[ -f "$source_snapshot/flake.nix" && -f "$source_snapshot/rust-toolchain.toml" ]] || {
  printf '%s\n' 'error: filtered source snapshot is incomplete' >&2
  exit 2
}
printf '%s\n' "$source_snapshot"
