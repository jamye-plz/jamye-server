set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

# Enter the pinned development environment. This never starts a service.
shell:
    nix develop path:.

# List task-owned command cards.
cards:
    @find docs/commands -type f -name '*.md' -not -name 'README.md' | LC_ALL=C sort

# Dispatch exactly one task-owned script from inside the Nix devShell.
task task_id card:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${IN_NIX_SHELL:-}" ]]; then
      printf '%s\n' 'error: enter with `nix develop path:.` before dispatching a task' >&2
      exit 2
    fi
    if [[ ! "{{task_id}}" =~ ^task-[0-9]+[a-z]?$ ]] || [[ ! "{{card}}" =~ ^[a-z0-9][a-z0-9-]*$ ]]; then
      printf '%s\n' 'error: invalid task or card name' >&2
      exit 2
    fi
    script="scripts/tasks/{{task_id}}/{{card}}.sh"
    if [[ ! -f "$script" ]]; then
      printf 'error: card script not found: %s\n' "$script" >&2
      exit 2
    fi
    bash "$script"

# Verify or create dependency locks through task-1's guarded implementation.
locks action="verify":
    @just task task-1 "locks-{{action}}"

# Explicit local-only rootless Podman lifecycle. Reset requires an extra guard.
infra action="status":
    @just task task-1 "infra-{{action}}"
