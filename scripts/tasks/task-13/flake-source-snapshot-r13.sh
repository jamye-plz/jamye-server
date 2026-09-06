#!/usr/bin/env bash
set -euo pipefail

if (($# != 0)); then
  printf '%s\n' 'error: flake-source-snapshot-r13.sh accepts no arguments' >&2
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

# Do not pass the live checkout as a relative path flake URL: that imports the
# entire mutable working tree, including ignored local state such as target/,
# .git/, and .agents/. This explicit filter includes current untracked source
# inputs without consulting Git or its index.
source_snapshot="$({
  TASK13_SOURCE_ROOT="$repo_root" nix eval --impure --raw --expr '
    let
      root = builtins.toPath (builtins.getEnv "TASK13_SOURCE_ROOT");
      rootString = toString root;
      rootFiles = [
        "AGENTS.md"
        "CLAUDE.md"
        "Cargo.lock"
        "Cargo.toml"
        "Justfile"
        "README.md"
        "compose.yaml"
        "deny.toml"
        "flake.lock"
        "flake.nix"
        "rust-toolchain.toml"
        ".env.example"
      ];
      relativePath = path:
        let
          value = toString path;
          prefixLength = (builtins.stringLength rootString) + 1;
        in if value == rootString then "" else builtins.substring prefixLength ((builtins.stringLength value) - prefixLength) value;
      included = path: type:
        let
          relative = relativePath path;
          allowedTree = (builtins.match "^(src|tests|migrations|contracts|scripts|nix|data|production_composition|docs)(/.*)?$" relative) != null;
          blockedNamedSegment = (builtins.match "(^|.*/)(target|result|[.]git|[.]agents)(/.*|$)" relative) != null;
          hiddenSegment = (builtins.match "(^|.*/)[.][^/]+(/.*|$)" relative) != null;
          environmentSegment = (builtins.match "(^|.*/)[.]env[^/]*(/.*|$)" relative) != null;
          docsPlans = (builtins.match "^docs/plans(/.*|$)" relative) != null;
        in type != "symlink" && (
          relative == ""
          || builtins.elem relative rootFiles
          || (allowedTree && !(blockedNamedSegment || hiddenSegment || environmentSegment || docsPlans))
        );
    in toString (builtins.path {
      path = root;
      name = "jamye-server-task13-r13-flake-source";
      filter = included;
    })
  '
} )"

[[ "$source_snapshot" == /nix/store/* ]] || {
  printf '%s\n' 'error: filtered source snapshot is not a Nix store path' >&2
  exit 2
}
[[ -d "$source_snapshot" && -f "$source_snapshot/flake.nix" && -f "$source_snapshot/rust-toolchain.toml" ]] || {
  printf '%s\n' 'error: filtered source snapshot is incomplete' >&2
  exit 2
}

# This helper's stdout is deliberately only the validated path, for callers
# that must preserve a command's original status through command substitution.
printf '%s\n' "$source_snapshot"
