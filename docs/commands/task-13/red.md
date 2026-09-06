# Task-13 flake-shape contracts

> Historical record only. R16 generation
> `task13-r16-g0-f4-coverage-cli-compat-20260906` is the sole active Task-13
> authority. No R11/R12 flake-shape card in this file is current; active R16
> receipt and shape cards are documented in `flake-check.md`. The accepted R15
> t13-s3 RED and active R16 coverage GREEN card are documented in `coverage.md`.

## Historical R11 evidence

Run this only from the repository's active Nix devShell:

```bash
just --justfile scripts/tasks/task-13/mod.just red
```

The card first rejects a stale or expanded Task-4b Redis-publish completion
token, then prints the immutable lowercase SHA-256 for
`tests/nix/task-13-assertion-manifest.json`. The coordinator must copy that
literal plus exactly one trailing LF into
`.agents/results/task-13-s1-manifest-digest-20260902-090159.txt`. The owned
lock and future coordinator record reject CRLF, split lines, and every trailing
byte; their only valid contents are `digest + LF`.

The baseline is valid only when it reports exactly these three failures:

1. `top-level flake output key-set mismatch (missing nixosModules)`
2. `packages.<system> key-set mismatch (extra default)`
3. `checks.<system> key-set mismatch (missing contract-drift)`

`per-system output family system key-sets exact` must pass, and the module
child check must say it was blocked/skipped because the parent is absent. The
script does not evaluate or serialize `nixosModules.default`.

Before any probe, the card creates one explicit filtered Nix source snapshot.
The first small `copying ... to the store` line can therefore be normal: it
contains only the approved current source/configuration inputs (including
untracked approved files) and is independent of Git staging. It excludes
every `target`/`result`/`.git`/`.agents` path segment, every hidden path
segment (including `.mcp.json`), `docs/plans/`, symlinks, and every `.env*`
file other than the root `.env.example`; an extended full-workspace copy is
invalid. Interrupting during that initial snapshot is safe: it creates no
repository file or coordinator record.

After the coordinator has recorded the digest, later Task-13 cards use the
bare active-devShell form `just --justfile scripts/tasks/task-13/mod.just
<recipe> <recorded-digest>`. `guarded-just.sh` rejects any mismatch among the
current manifest, its lock, the fixed record, and that argument before it
delegates only to its immutable manifest-derived Justfile/recipe allowlist.

The following R11 rerun is retained only for historical evidence. It is not an
active Task-13 acceptance card after the aarch64-linux expansion:

```bash
just --justfile scripts/tasks/task-13/mod.just red <recorded-digest>
```

That single argument enters the historical `guarded-just.sh` self-dispatch,
whose nested `red` has no argument and therefore runs the bootstrap shape
contract once. This avoids recursive dispatch and rejects a missing, malformed,
or mismatched coordinator record before the Nix action.

## Historical R12 aarch64-linux RED

R12 is already authored at
`tests/nix/task-13-assertion-manifest-aarch64-linux.json` with its separate lock
and coordinator record. This first card only verifies the canonical bytes,
generation identity, and exact manifest/lock/record digest equality, then
derives and prints that coordinator-bound digest. The caller-supplied four-way
proof belongs to the following shape card. Despite its name, this receipt card
performs zero writes and never generates, relocks, or rebaselines the manifest:

```bash
just --justfile scripts/tasks/task-13/mod.just aarch64-linux-manifest
```

Copy the printed R12 digest into the active shape card:

```bash
just --justfile scripts/tasks/task-13/mod.just aarch64-linux-shape <r12-recorded-digest>
printf 'task_13_s2_aarch64_linux_red_exit=%s\n' "$?"
```

The `mod.just` shape recipe is thin outer argv transport only. The guard itself
revalidates the current G0 completion token and four-way active generation
digest before snapshot creation. The active manifest binds exact SHA-256
values for the dedicated `r12-shape.just`, `guarded-just.sh`,
`flake-shape-aarch64-linux.sh`, and `flake-source-snapshot.sh`. The guard
verifies all four live immutable sources, executes one separately hash-verified
private copy of the snapshot helper, and creates exactly one filtered
`/nix/store` snapshot. It then verifies the same four source hashes and
manifest bytes inside that snapshot, revalidates the live inputs, and invokes
the internal `aarch64-linux-shape-immutable` recipe from the snapshot copy of
`r12-shape.just` while the snapshot is the working directory. The shape script
consumes that supplied snapshot and never creates a second one.

The internal immutable recipe is not an operator card. A content mutation or
path replacement of `r12-shape.just` after initial validation is a hard exit-2
block before nested dispatch; the R12 static fixture fixes the expected nested
dispatch count at zero. Mutable `mod.just` is never reopened as a trusted
nested source, so legitimate t13-s3/t13-s4/t13-s5 recipe additions do not
require an R12 rebaseline. There is no live-recipe fallback.

Before `flake.nix` is changed, valid RED evidence has exactly one failure:
`supported-system matrix missing aarch64-linux`. Its detail must name, in
order, `packages`, `devShells`, `checks`, and `supportedSystems`, each as
missing only `aarch64-linux`. The script probes child inventories only for
systems present in every family, so it never forces a missing aarch64-linux
child. All existing-system child, top-level, module-presence, and toolchain
assertions must pass; disposition is `BASELINE_RED (one named failure)` and
exit is `1`.

Any other failure inventory, the historical R11 digest, a noncanonical R12
manifest, or a digest/lock/record mismatch is invalid evidence and blocks
before the shape action.

## Accepted R12 RED baseline

The user-run R12 receipt completed with exit 0 and printed digest
`e978c88ddd19ccb5de750597c1fdea1fc126ed5e2e597ece055b7a6316c6c4c6`.
The user-run shape card then exited 1 with exactly one named failure,
`supported-system matrix missing aarch64-linux`, and disposition
`BASELINE_RED (one named failure)`. Its ordered nested detail showed packages,
devShells, checks, and supportedSystems each missing only `aarch64-linux`;
missing aarch64-linux child outputs were not probed or forced.

That baseline was accepted before the system-list implementation. It remains
historical RED evidence and must not be regenerated from the post-change tree.
