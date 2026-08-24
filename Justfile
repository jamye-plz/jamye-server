set shell := ["bash", "-euo", "pipefail", "-c"]
set default-list

# M0 commands are namespaced and discoverable with `just task-1`.
mod task-1 'scripts/tasks/task-1/mod.just'

# Enter the pinned development environment. This never starts a service.
shell:
    nix develop path:.

# List task-owned command cards.
cards:
    @find docs/commands -type f -name '*.md' -not -name 'README.md' | LC_ALL=C sort
