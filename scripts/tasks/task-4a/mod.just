set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list
set working-directory := "../../.."

[private]
require-nix-shell:
    @if [[ -z "${IN_NIX_SHELL:-}" ]]; then printf '%s\n' 'error: enter with `nix develop path:.` before running task-4a' >&2; exit 2; fi

# Prove the task-4a behavior target fails only because messaging is not implemented yet.
red: require-nix-shell
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -e src/domain/messaging/mod.rs ]]; then
      printf '%s\n' 'error: RED must run before the task-4a messaging implementation exists' >&2
      exit 2
    fi
    cargo test --locked --test messaging reliable_messaging_surface_is_absent_before_task_4a -- --exact --nocapture
