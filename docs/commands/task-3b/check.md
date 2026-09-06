# Task 3b — verify disposable C0 and committed contract profile

## Purpose

Run the contract integration test and the exact existing
`contract_generation::allowed_provenance_variants_are_deterministic_and_emit_release_candidate_manifest`
production-composition assertion, regenerate generic C0 into a disposable
directory, and independently verify both the disposable C0 tree and the
committed contract tree with their manifest-aware provenance, checksum,
artifact-path, byte, symlink, and contribution rules. The command does not
compare those two trees directly: C0 and the committed release-candidate
profile intentionally have different inventories.

Before executing the release-candidate assertion, `check` lists the same exact
selector from the `production_composition` target and requires exactly one
literal `<selector>: test` inventory entry. This prevents Cargo's zero-test
exact-filter success or an ambiguous duplicate inventory from becoming a false
pass.

## Preconditions

- Use an active pinned development shell.
- Ensure the committed `contracts/` tree is present. Do not run the legacy
  task-3b `generate` card as a prerequisite: it materializes generic C0 and
  refuses when the current release-candidate extra files are present; it does
  not replace the committed release-candidate profile.

## User-run command

```bash
just --justfile scripts/tasks/task-3b/mod.just check
status=$?
printf 'task_3b_check_exit=%s\n' "$status"
(exit "$status")
```

## Side effects

- Compiles/runs the task-3b Rust test and generator.
- Creates a temporary directory and removes it through a guarded trap.
- Does not modify `contracts/`, the Git index, another repository, or remote state.
- The legacy `generate` recipe remains available for its separate C0
  materialization purpose; it is not a prerequisite for `check`, and current
  release-candidate extra files make it refuse rather than modify `contracts/`.

## Expected result

The C0 integration test and exact release-candidate assertion pass, the
disposable C0 and committed profile both verify, the final line is `contract
artifact allowlist, bytes, provenance, and checksum match`, and the exit code
is `0`.
