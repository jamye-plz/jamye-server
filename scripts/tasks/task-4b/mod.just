set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter with `nix develop path:.` before running task-4b' >&2; exit 2; fi

# Prove the task-4b target fails only because the realtime delivery surface is absent.
red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -e src/application/realtime/mod.rs ]]; then
      printf '%s\n' 'error: RED must run before the task-4b realtime implementation exists' >&2
      exit 2
    fi
    cargo test --locked --test realtime realtime_surface_is_absent_before_task_4b -- --exact --nocapture
