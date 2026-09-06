# Task-13 flake checks

The sole active successor is R16 generation
`task13-r16-g0-f4-coverage-cli-compat-20260906`, bound to manifest digest
`d177dfdbda7f25aac02857e6c73d9fb86c378699d1c94ce1ce648b4df6656e79`.
Run these bare cards from the repository's already-active Nix devShell only
after the final R16 implementation reviews pass. Run each card separately and
retain its printed exit status.

Receipt:

```bash
just --justfile scripts/tasks/task-13/mod.just r16-manifest
status=$?
printf 'task_13_s2_r16_manifest_exit=%s\n' "$status"
(exit "$status")
```

Exact three-system shape:

```bash
just --justfile scripts/tasks/task-13/mod.just aarch64-linux-shape-r16 d177dfdbda7f25aac02857e6c73d9fb86c378699d1c94ce1ce648b4df6656e79
status=$?
printf 'task_13_s2_r16_shape_green_exit=%s\n' "$status"
(exit "$status")
```

Native flake verification:

```bash
just --justfile scripts/tasks/task-13/mod.just guarded-flake-local-r16 d177dfdbda7f25aac02857e6c73d9fb86c378699d1c94ce1ce648b4df6656e79
status=$?
printf 'task_13_s2_r16_guarded_flake_local_exit=%s\n' "$status"
(exit "$status")
```

R16 consumes only its two immutable authority snapshots, G0-F4 token, sole
token-integrity review, manifest/lock/record, and four frozen R16 sources. It
never opens mutable canonical plan/requirements or an R11-R15 authority
artifact. Its exact class partition is 3 immutable-snapshot plus 16
live-worktree pairs, totaling 19. `coverage-r16-exec` is the only Task-13
coverage pair; the public `coverage-r16` name is argv transport only.

Immutable R16 shape and Task-1 flake delegates retain the filtered source
snapshot path. Live targets run only from an adjacent read-only per-dispatch
copy after final device, inode, and SHA-256 revalidation. The R16 coverage
executor keeps its environment-file parser inside that guard-bound copy and
has no mutable code auxiliary. The R16 executable coverage-executor fixture
drives both content mutation and path replacement through the production
validator in disposable repository copies, fixing exit 2 and zero nested
dispatch without touching real repository bytes. The fixture remains unrun.
The mutable Task-13 `mod.just` is not a frozen source.

These R16 cards are authored but remain user-unrun at this point. Receipt,
shape, native flake output, final implementation reviews, and every later
coverage/descriptor/FV result are external to the token and manifest digest.

## Historical R15 record

R15 generation `task13-r15-g0-f3-coverage-authority-20260905` remains bound to
historical digest
`e4332daf6237645c04e6b473796aeccd7a44481a6c754f4bd364c6ee73d9bbc1`.
Its receipt, three-system shape, and host-compatible guarded flake card passed.
Its accepted RED and three failed GREEN attempts remain immutable evidence;
R15's `--no-report --no-clean` collection contract is not runnable acceptance
authority. Do not rerun its receipt, shape, flake, RED, or GREEN cards. R16
supersedes only the active authority and private coverage GREEN binding.

## Historical R14 record

The following R14 cards and descriptions are retained for audit only. They no
longer authorize t13-s2 through t13-s5 or FV01 through FV09. R14 generation
`task13-r14-g0-f2-authority-snapshots-20260905` remains bound to historical
manifest digest
`f17bc14e38edc0d3d77d317a486612a03535423093d47acd4855151592c927c9`.

```bash
just --justfile scripts/tasks/task-13/mod.just r14-manifest
just --justfile scripts/tasks/task-13/mod.just aarch64-linux-shape-r14 f17bc14e38edc0d3d77d317a486612a03535423093d47acd4855151592c927c9
just --justfile scripts/tasks/task-13/mod.just guarded-flake-local-r14 f17bc14e38edc0d3d77d317a486612a03535423093d47acd4855151592c927c9
```

`r14-manifest` is zero-argument, zero-mutation receipt-only verification. It
pins the append-only G0-F2 token and sole token-integrity review, validates the
two immutable authority snapshots without reading or comparing their mutable
canonical issuance sources, and checks canonical manifest bytes, exact
manifest/lock/coordinator-record equality, and all four frozen R14 source
hashes.

`aarch64-linux-shape-r14` is GREEN-only. It requires the exact ordered
`aarch64-darwin,aarch64-linux,x86_64-linux` matrix and exact package, devShell,
check, and module inventories; any missing or extra value is invalid evidence.
Only its exit-0 GREEN receipt authorizes the subsequent
`guarded-flake-local-r14` card.

The R14 guard validates the exact 3 immutable-snapshot plus 15 live-worktree
pair partition, rejects symlinks and physical-path escape, and revalidates all
applicable live and snapshot bytes immediately before its sole nested dispatch.
The immutable pairs are R14 shape and unchanged Task-1 `flake-local` and
`flake-linux`. A single `builtins.path` snapshot includes the explicit source
trees and root `.env.example`, while excluding symlinks, `target`, `result`,
`.git`, `.agents`, other hidden/environment paths, and `docs/plans`. Mutable
Task-13 `mod.just` remains quoted outer argv transport and is not frozen.

For a live-worktree pair, the guard binds the selected target's device, inode,
and SHA-256, creates a read-only per-dispatch copy beside that Justfile, and
revalidates both identities and byte hashes at the final pre-dispatch point.
Only the verified copy is passed to Just. Keeping it in the same physical
directory preserves the target's relative `set working-directory` behavior;
EXIT/HUP/INT/TERM cleanup removes it and the nested recipe's exit status is
returned unchanged. No live target byte is added to long-lived R14 authority.

Final R14 implementation CCR reviews were an external pre-card gate. They and
the user receipt, shape result, guarded-flake result, descriptor/FV outputs,
and final handoff hashes are not token, guard, or manifest inputs.

## Historical R13 record

The following R13 commands and outcomes are retained for audit only; they are
not valid active G2/t13-s2 evidence.

The historical generation was
`task13-r13-g0-f1-format-drift-20260905`, bound to manifest digest
`0db960caab9a328025e4d509786a82ecc0f181f5c31fc4c098c5b1720b6c26d3`.
Its preserved command spellings are shown only as provenance:

```bash
just --justfile scripts/tasks/task-13/mod.just r13-manifest
just --justfile scripts/tasks/task-13/mod.just aarch64-linux-shape-r13 0db960caab9a328025e4d509786a82ecc0f181f5c31fc4c098c5b1720b6c26d3
just --justfile scripts/tasks/task-13/mod.just guarded-flake-local-r13 0db960caab9a328025e4d509786a82ecc0f181f5c31fc4c098c5b1720b6c26d3
```

`r13-manifest` is zero-argument and receipt-only. It verifies the exact
G0-F1 token and its clean static review, canonical manifest bytes, exact
manifest/lock/coordinator-record equality, and all four frozen R13 source
hashes before echoing the generation and digest. It writes nothing.

`aarch64-linux-shape-r13` is GREEN-only. It requires the exact ordered
three-system matrix and exact package, devShell, check, and module inventories;
any missing or extra value is invalid evidence. R13 cannot manufacture or
accept a new RED. Its former exit-0 GREEN receipt does not authorize an R14
dispatch.

The R13 guard accepts only the literal R13 generation. It independently
revalidates G0-F1, the four-way manifest/lock/record/caller digest, the exact
four frozen R13 sources, the exhaustive 3 immutable-snapshot plus 15
live-worktree pair partition, and physical non-symlink path identity.
Task-1's unchanged Justfile is separately hash-bound only for its
`flake-local`/`flake-linux` delegates. Mutable Task-13 `mod.just` remains
quoted outer argv transport and is not a frozen source.

The user-owned R13 receipt and GREEN-only shape cards both passed with the
exact digest above. The first guarded native flake card also validated the
R13 authority, emitted its immutable snapshot path, and completed the exact
three-system `flake show`, but `nix flake check` stopped at
`checks.aarch64-darwin.contract-drift`.

That failure was a profile mismatch: the prior Task-3b checker generated the
preserved 16-artifact C0 profile and compared it with the current Task-12
release-candidate tree (21 generated artifacts plus 18 read-only owner
contributions). The user approved Option A: retain the disposable C0
generation/verification and verify the committed tree through its own
manifest-selected profile, while removing only the invalid cross-profile
equality assertion. G0-F2/R14 supersedes the predecessor authority rather than
re-running or relocking it. The accepted RED evidence is recorded in
`.agents/results/task-13-s2-r13-guarded-flake-local-contract-drift-user-evidence-20260905.md`.

## Historical R12 record

The mutable Task-13 adapter no longer exposes any R11/R12 execution card. The
superseded `red`, `aarch64-linux-manifest`, `aarch64-linux-shape`, and
`guarded-flake-local` recipes, together with their private legacy G0 helper,
were removed. They must not be invoked or treated as acceptance evidence.

Historical material remains byte-preserved for audit only:

- R11 manifest, lock, and coordinator record;
- R12 aarch64-linux manifest, lock, and coordinator record;
- the R12 frozen shape Justfile, guard, shape script, and source-snapshot
  helper; and
- the original and G0-F1 prerequisite tokens and their evidence records.

These artifacts are not active runtime dependencies. The mutable adapter may
retain historical spellings for audit compatibility, but the R16 guard rejects
every predecessor generation as active input. Historical user outputs remain
records of predecessor RED, GREEN, and native flake-check attempts; they confer
no current acceptance. Only the three R16 cards at the top of this document
form the active t13-s2 sequence, and they remain user-unrun until the R16
implementation review gate closes.
