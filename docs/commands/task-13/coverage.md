# Task-13 coverage gate

Run Task-13 cards from the repository's already-active Nix devShell. The sole
active authority is R16 generation
`task13-r16-g0-f4-coverage-cli-compat-20260906`, with recorded manifest digest
`d177dfdbda7f25aac02857e6c73d9fb86c378699d1c94ce1ce648b4df6656e79`.
An outer `nix develop path:. --command` wrapper is not valid evidence.

## Required order

Run each card only after the preceding card has produced its expected success
evidence. Do not combine or skip cards.

### 1. R16 immutable-authority receipt

```bash
just --justfile scripts/tasks/task-13/mod.just r16-manifest
status=$?
printf 'task_13_s2_r16_manifest_exit=%s\n' "$status"
(exit "$status")
```

Expected: `ACTIVE_R16_MANIFEST_VERIFIED`, manifest digest
`d177dfdbda7f25aac02857e6c73d9fb86c378699d1c94ce1ce648b4df6656e79`,
and exit 0.

### 2. Three-system shape

```bash
just --justfile scripts/tasks/task-13/mod.just aarch64-linux-shape-r16 d177dfdbda7f25aac02857e6c73d9fb86c378699d1c94ce1ce648b4df6656e79
status=$?
printf 'task_13_s2_r16_shape_green_exit=%s\n' "$status"
(exit "$status")
```

Expected: exact `aarch64-darwin,aarch64-linux,x86_64-linux` package,
devShell, check, and `supportedSystems` matrices; zero named failures; exit 0.

### 3. Local flake realization

```bash
just --justfile scripts/tasks/task-13/mod.just guarded-flake-local-r16 d177dfdbda7f25aac02857e6c73d9fb86c378699d1c94ce1ce648b4df6656e79
status=$?
printf 'task_13_s2_r16_guarded_flake_local_exit=%s\n' "$status"
(exit "$status")
```

Expected: the R16-filtered source snapshot exposes the exact flake surface and
the host-compatible checks pass. Incompatible systems may be reported as
omitted by the host; their realization remains a later builder-matrix gate.

### 4. Coverage GREEN

```bash
just --justfile scripts/tasks/task-13/mod.just coverage-r16 d177dfdbda7f25aac02857e6c73d9fb86c378699d1c94ce1ce648b4df6656e79
status=$?
printf 'task_13_s3_r16_coverage_green_exit=%s\n' "$status"
(exit "$status")
```

Expected: exactly 1 PASS, 0 FAIL, 0 SKIP, and exit 0. The line-coverage floor
is 80 percent.

## Active R16 coverage contract

The public `coverage-r16` recipe is a thin argv transport. The R16 guard may
select only the private `coverage-r16-exec` pair from a verified read-only
adjacent copy. Before coverage work, the executor validates the exact R16
generation and digest, the bound Justfile copy, Nix-store tool paths, Rust
`1.98.0`, cargo-llvm-cov `0.9.0`, both LLVM coverage tools, and independently
derived default/all-feature Cargo target inventories.

The frozen command sequence is:

1. Clean `target/task-13-llvm-cov` exactly once.
2. Collect the locked/offline default-feature workspace with `--all-targets
   --no-report`.
3. In the same target directory, collect the locked/offline fixture-enabled
   all-feature workspace with `--all-targets --all-features --no-report`.
4. Produce one JSON summary with `--fail-under-lines 80`.

For pinned cargo-llvm-cov `0.9.0`, `--no-report` is the sole collection
retention flag. R16 deliberately omits `--no-clean`: that version rejects the
R15 combination `--no-report --no-clean`. The executor proves that default raw
profiles remain regular files after the second collection and that the total
profile count grows before the sole report. No intervening clean, report, or
target-directory replacement is accepted.

Evidence is written under
`target/task-13-coverage-evidence/<r16-digest>` and includes both mode logs,
target metadata, raw-profile inventories, the JSON summary, and report stderr.
The transcript also emits tool provenance, target inventories, retention
counts, the JSON report, and a human-readable summary.

The manifest-bound fixture
`tests/nix/coverage-executor-toctou-r16.sh` exercises content mutation and path
replacement at the production post-copy barrier in disposable repository
copies. Each case requires guard exit 2, zero nested dispatch, and zero
persistent repository writes. It is not a substitute for the user-run GREEN
card.

## Historical R15 evidence

R15 is immutable historical evidence and is not an active runtime fallback.
Its accepted RED ran once with exactly two failures, no PASS, no SKIP, and a
non-zero exit:

1. `coverage recipe missing`
2. `cargo-llvm-cov devShell tool missing`

Do not rerun the R15 RED or GREEN cards. The three R15 GREEN attempts remain
separate failure evidence:

1. stale devShell: `cargo-llvm-cov` unavailable;
2. incorrect direct version-probe argv;
3. pinned cargo-llvm-cov `0.9.0` rejecting `--no-report --no-clean`.

The third attempt completed the initial clean but ran no collection test and
no report. None of the three attempts is a coverage result. R16 supersedes
only the invalid coverage command authority and private GREEN executor binding;
R15 files, hashes, and evidence remain unchanged.
