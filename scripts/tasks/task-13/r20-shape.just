set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set positional-arguments
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter the repository Nix devShell before running task-13' >&2; exit 2; fi

# Frozen t13-s2 nested implementation. The mutable Task-13 mod.just may add
# later sprint recipes, but the active guard dispatches this exact Justfile
# only from its verified immutable R20 source snapshot.
aarch64-linux-shape-r20-immutable: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    generation_id='task13-r20-g0-f8-token-review-path-consistency-20260907'
    [[ "${TASK13_ASSERTION_GENERATION:-}" == "$generation_id" ]] || { printf '%s\n' 'error: active R20 shape requires guarded generation selection' >&2; exit 2; }
    [[ "${TASK13_ASSERTION_DIGEST:-}" =~ ^[0-9a-f]{64}$ ]] || { printf '%s\n' 'error: active R20 shape guard digest receipt is missing' >&2; exit 2; }
    [[ "${TASK13_FLAKE_SOURCE_SNAPSHOT:-}" == /nix/store/* && -d "$TASK13_FLAKE_SOURCE_SNAPSHOT" ]] || { printf '%s\n' 'error: active R20 shape snapshot receipt is missing or invalid' >&2; exit 2; }
    [[ "$(pwd -P)" == "$TASK13_FLAKE_SOURCE_SNAPSHOT" ]] || { printf '%s\n' 'error: active R20 shape is not running from the guarded snapshot' >&2; exit 2; }
    [[ "$(sha256sum tests/nix/task-13-assertion-manifest-r20-g0-f8.json | awk '{print $1}')" == "$TASK13_ASSERTION_DIGEST" ]] || { printf '%s\n' 'error: active R20 manifest changed after guarded dispatch' >&2; exit 2; }
    exec bash tests/nix/flake-shape-aarch64-linux-r20.sh

