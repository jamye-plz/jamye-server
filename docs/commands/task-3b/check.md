# Task 3b — verify C0 contract drift

## Purpose

Run the contract integration test, regenerate C0 into a disposable directory, verify manifest provenance/checksum, reject missing or extra artifacts, and byte-compare the generated and committed allowlists without using the Git index.

## Preconditions

- Enter the pinned shell with `nix develop path:.`.
- Run the task-3b generate card once so `contracts/` exists.

## User-run command

```bash
just --justfile scripts/tasks/task-3b/mod.just check
printf 'task_3b_check_exit=%s\n' "$?"
```

## Side effects

- Compiles/runs the task-3b Rust test and generator.
- Creates a temporary directory and removes it through a guarded trap.
- Does not modify `contracts/`, the Git index, another repository, or remote state.

## Expected result

The integration tests pass, both manifests verify, the final line is `contract artifact allowlist, bytes, provenance, and checksum match`, and the exit code is `0`.
