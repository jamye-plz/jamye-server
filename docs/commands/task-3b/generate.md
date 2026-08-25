# Task 3b — generate C0 contracts

## Purpose

Generate the deterministic C0 OpenAPI, realtime schemas/protocol, fixtures, and manifest into `contracts/`. The generator reads only `src/contract_generation/provenance.json`; it does not inspect Git HEAD, a tag, the clock, or the host.

## Preconditions

- D1=A, D8=A, and D13=A are already locked by the user.
- Enter the pinned shell with `nix develop path:.`.
- `contracts/` must contain no file outside the generator allowlist. The generator refuses to delete unknown files.

## User-run command

```bash
just --justfile scripts/tasks/task-3b/mod.just generate
printf 'task_3b_generate_exit=%s\n' "$?"
```

## Side effects

- Creates or replaces only allowlisted files under `contracts/`.
- Does not create or modify `jamye-app/contract.lock`.
- Does not stage, commit, tag, or push.

## Expected result

The command reports the number of deterministic artifacts and exits `0`. Review the generated tree before running the check card.
