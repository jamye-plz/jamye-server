# M11b / Task-13 durable requirements

Session: `20260902-090159`

## User directive

- `Task-13 ultrawork PLAN Step 1 — PM authoring only`
- Baseline: signed Task-12 commit `16249aa0c664a8f186c1de0b867d25ffb9fcbd43`; worktree clean.
- Produce a durable Task-13 requirements document and executable implementation
  plan; no implementation, review of this plan, product-file modification,
  build/test/evaluation execution, commit, push, deploy, migration, or
  production-state access in this turn.
- The user owns every executable Nix/Cargo/build/test/fmt/Clippy/coverage/
  module-evaluation command. This turn may specify exact future user-run
  command cards but must not execute them.
- No Serena onboarding, no Serena memory creation this turn.
- No self-review of this plan (fresh isolated reviewers run PLAN Steps 2-4 in a
  later turn); this turn ends after authoring.

## Locked behavior consumed (not re-decided here)

Task-13 materializes no new `decision_registry` entry. It consumes frozen
evidence from:

- **D10=A** (task-11) — account-deletion disposition: shared content/media
  anonymized by author tombstone, credential/profile/push/notification/read/
  membership state deleted, invites revoked, durable object-cleanup intents
  recorded. Task-13 exposes no new deletion behavior; it only documents that
  the cleanup runner's lease/timing controls (already implemented by task-11)
  remain non-secret and configurable via `environmentFile`, not the Nix store.
- **D11=B** (task-8) — private MinIO bucket lifecycle: the API's
  `ensure_bucket` three-branch behavior (HEAD success no-op; exact
  404/NoSuchBucket create; non-404 typed propagate/degraded) is the sole
  bucket-lifecycle owner. Task-13's NixOS module therefore creates no
  bucket-provisioning oneshot unit under any configuration.
- **D12=A** (task-5) — mobile OAuth Authorization Code + PKCE S256 exchange.
  Task-13 exposes no OAuth behavior; it only ensures `AuthConfig`'s existing
  non-secret client-id/redirect keys (not secrets) may be supplied via
  `environmentFile` at the systemd-unit level, without introducing a new
  option surface for OAuth itself.

Only the non-secret key *names* for these three decisions may be documented in
`.env.example`, module options, or `docs/deployment-nix.md`. No secret value,
production identifier, or credential is authored anywhere in Task-13.

## Locked scope for Task-13 (from `plan-20260822-200110.json` task-13 +
`static_transaction_orchestration`/`legacy_scope_lock` boundary notes)

Verbatim acceptance criteria from the master plan's `task-13` object (preserved
without weakening; see also `.agents/state/memories/task-board.md` task-13
entry for the current-session restatement):

1. Flake outputs exactly `devShells.<system>.default`,
   `packages.<system>.api`, `packages.<system>.worker`, `checks.<system>.*`,
   and `nixosModules.default` for `aarch64-darwin` and `x86_64-linux`;
   `rust-toolchain.toml` remains the sole exact Rust value source.
2. The package matrix builds `api` and `worker` for both supported systems.
   PF2 verifies an `x86_64-linux` builder before the production lane; absence
   is an explicit blocker and no Linux success is claimed.
3. Default-feature and `--all-features` Rust tests are separate canonical
   recipes. The coverage recipe includes library, binaries, and integration
   test targets and enforces `>=80%` without narrowing to `--lib --bins`.
4. Task-13 owns exact Nix package/module/flake command docs and scripts under
   `docs/commands/task-13/` and `scripts/tasks/task-13/`. The user is already
   inside the declared devShell, so every executable Task-13 user card uses
   bare `just --justfile ...`; task-13 does not edit `README.md` or the root
   `Justfile`. The immutable final-verify inventory retains its existing
   dispatcher command strings and is not an outer user-card wrapper.
5. NixOS module evaluation proves api/worker packages become native systemd
   `ExecStart` programs, `listenAddress` and `environmentFile` are wired
   exactly, `migrations.mode` defaults `manual` with optional api-prestart
   only, and `objectStorage.createLocally` defaults `false`.
6. When `objectStorage.createLocally=true`, the module enables native NixOS
   `services.minio` with configurable package/S3 listen address, loopback-only
   console address, required `rootCredentialsFile`, and non-secret public
   presign endpoint/bucket options. It creates no bucket oneshot (D11=B keeps
   the API's `ensure_bucket` path the sole bucket-lifecycle owner). API/worker
   use `after`+`wants` rather than `requires` so MinIO failure degrades only
   storage endpoints; text-only PostgreSQL paths remain available.
7. The module and flake never create PostgreSQL, Redis, Podman, host/domain/
   volume, ingress, secret, monitoring, backup, or restore resources. Homelab
   owns those boundaries and all production values; MinIO root/app credentials
   remain `environmentFile`/SOPS-owned and never enter the Nix store.
8. `.env.example` and module options expose non-secret validated
   lease/timeouts for Redis publish, Expo delivery, and object deletion. Each
   short-I/O timeout plus safety margin is below its lease; values contain no
   production identifier or secret.
9. `checks.<system>.contract-drift` invokes the task-3b canonical
   explicit-provenance checker; module evaluation and package checks are
   reproducible and use no ambient tool or unpinned installer.
10. Task-13 consumes the frozen task-11 D10 disposition, task-8 D11=B storage
    boundary, and task-5 D12 auth configuration; only their non-secret key
    names and validated options are represented. Module evaluation covers
    local-MinIO disabled/enabled branches, missing-root-credential rejection,
    loopback console binding, nonblocking MinIO service ordering, and absence
    of a bucket-provisioning unit.
11. `docs/deployment-nix.md` records rootless Podman Compose as
    local-development-only, native NixOS/systemd production deployment,
    optional native MinIO co-deployment, manual migration policy, homelab
    responsibility, and the SigV4 rule forbidding unconditional MinIO `Host`
    rewriting. Production host/domain/volume, secret values, ingress,
    activation, and deploy commands remain outside this repository and plan.

## Current repository state observed (read-only inspection this turn)

- `flake.nix` already declares `supportedSystems = ["aarch64-darwin"
  "x86_64-linux"]`, per-system `devShells.default`, `packages.{api,worker,
  default}`, and `checks.{api,worker,cargo-fmt,cargo-clippy,
  cargo-test-default,cargo-test-all-features,architecture}`, consuming
  `rust-toolchain.toml` via `fenix.fromToolchainFile` as its sole Rust
  declaration. **No build, check, or evaluation of this flake has been run by
  this plan-authoring turn; no pass is claimed for any existing attribute.**
  This scaffold predates Task-13 and has never been verified against the
  locked acceptance criteria above.
- `nixosModules.default` and `checks.<system>.contract-drift` are absent from
  `flake.nix`.
- `nix/`, `tests/nix/`, `docs/commands/task-13/`, `scripts/tasks/task-13/`, and
  `docs/deployment-nix.md` do not exist.
- `rust-toolchain.toml` declares `channel=1.98.0`, `profile=minimal`,
  `components=[clippy,rust-analyzer,rust-src,rustfmt]`,
  `targets=[aarch64-apple-darwin,x86_64-unknown-linux-gnu]`. No `llvm-tools`/
  `llvm-tools-preview` component is present; the coverage sprint may need to
  add one (see assumptions).
- `.env.example` currently declares only M0 keys (`JAMYE_ENVIRONMENT`,
  `JAMYE_LISTEN_ADDR`, `JAMYE_SHUTDOWN_GRACE_SECONDS`,
  `JAMYE_READINESS_TIMEOUT_MS`, `DATABASE_URL`, `REDIS_URL`,
  `JAMYE_MINIO_HEALTH_URL`). No Redis-publish/Expo-delivery/object-deletion
  lease/timeout keys exist yet.
- `scripts/tasks/task-1/mod.just` already exposes `flake-local` (evaluate +
  `nix flake check` without writing the lockfile) and `flake-linux` (build
  `packages.x86_64-linux.{api,worker}` through an explicit Linux builder,
  treating an absent builder as a stated blocker). Task-13 must reuse, not
  duplicate, these two cards. Under the approved 2026-09-05 Option A split,
  t13-s2 extracts its s1 source filter into
  `scripts/tasks/task-13/flake-source-snapshot.sh`: exactly once before each
  guarded delegate it validates and creates one immutable filtered snapshot,
  emits its path, and runs unchanged Task-1 `flake-local` (and later
  `flake-linux`) entirely from that snapshot. It does not write/rebaseline the
  Task-13 assertion manifest, lock, or fixed digest record.
- `scripts/tasks/task-1/mod.just` also exposes stable `format-check`,
  `clippy`, `architecture`, and `aggregate` targets (added under the
  Task-12 G5 scope amendment) that Task-13's own quality gates should reuse
  rather than re-declare.
- `scripts/tasks/task-11/mod.just` shows the established per-task Just-module
  convention: `require-nix-shell` private recipe, one recipe per RED/GREEN
  card, identical RED/GREEN selectors where the underlying behavior (not the
  test set) changes between gates, and an explicit blocked/reserved recipe
  (`coverage`) with a deliberate non-zero exit and message pointing at the
  Task-12/13 handoff — this is the exact recipe Task-13 now closes.
- No coverage tool other than pinned `cargo-llvm-cov` is
  referenced anywhere in the repository outside the OMA workflow
  documentation's generic `coverage` vocabulary. Coverage tooling is
  greenfield for this task.
- The task-3b canonical explicit-provenance checker is
  `scripts/tasks/task-3b/contract-check.sh`, dispatched by
  `scripts/tasks/task-3b/mod.just`'s `check` recipe. It requires `IN_NIX_SHELL`
  and retains `cargo test --locked --test contract` for generic disposable C0
  determinism. Legacy generic C0 generation encountering current extra paths
  refuses; it does not overwrite the committed RC and is not an RC-check
  prerequisite. Before committed RC execution, one shared selector variable
  lists the same target/filter with `-- --exact --list` and requires exactly
  `contract_generation::allowed_provenance_variants_are_deterministic_and_emit_release_candidate_manifest: test`;
  zero/multiple matches or listing failure block and propagate, with no
  pipeline or command-substitution bypass. That unchanged selector is then
  verified only through `cargo test --locked --test production_composition
  contract_generation::allowed_provenance_variants_are_deterministic_and_emit_release_candidate_manifest
  -- --exact`: the existing manifest-aware path requiring committed RC stage,
  exactly 21 generated artifacts, and exactly 18 owner contribution files. It
  is not a byte-equality peer of a temporary C0 root.
  `checks.<system>.contract-drift` invokes this checker rather than
  reimplementing its logic. `contract-check.sh` propagates its native status
  only through `set -e`/normal shell semantics and retains the existing
  cleanup-only `trap 'rm -rf -- "$temporary_root"' EXIT` for its exact
  temporary root. It adds no status-receipt/status-print trap and does not
  merge status reporting into cleanup. The documented user/operator card
  captures the Just status, prints it, then `exit "$status"` returns that same
  status.

## Approved 2026-09-05 Option A amendment: snapshot-bound compile checks

This is a narrow t13-s2 plan amendment. It preserves exactly five sequential
sprints and the existing DAG, s1 manifest/lock/fixed digest, all existing
output inventories, 12/3/3 → 15/0/3 → 18/0/0 module-evaluation counts, and
all approval flags. It does not authorize implementation or command execution.

- `t13-s1` owns the deterministic source filter used by its flake-shape
  evidence. `t13-s2` extracts that filter into the one Task-13 helper
  `scripts/tasks/task-13/flake-source-snapshot.sh`; the helper validates,
  creates, and emits one immutable filtered snapshot before dispatch.
  `guarded-flake-local` then delegates unchanged Task-1 `flake-local` entirely
  from that snapshot. Final t13-s5 reuses the same helper branch for its one
  unchanged guarded Task-1 `flake-linux` delegate. There is no pre/post
  snapshot comparison: the handed-off store path is immutable.
- `checks.<system>.cargo-test-default` remains present and runs exactly
  `cargo test --locked --all-targets --no-run`. The separately retained
  `checks.<system>.cargo-test-all-features` runs exactly
  `cargo test --locked --all-targets --all-features --no-run`. Both Nix checks
  are compile-only; neither is service-backed runtime proof. Full service-backed
  runtime behavior remains Task-1 `aggregate`, then Task-13 coverage and the
  later final audit. This does not relax the >=80% coverage gate.
- Since the user is already in the devShell, executable user cards are bare
  `just --justfile ...` commands. No outer `nix develop path:. --command`
  wrapper is valid user-card evidence. Task-1 remains unchanged and is reached
  only through Task-13's guarded adapter plus the snapshot handoff.
- The byte-exact final-verify assertion manifest remains untouched: the nine
  FV IDs, command inventory, `dispatch_by_id`, and existing `platform-check`
  entry do not change. FV-03 must first load and validate the same prepared
  local test environment before that existing guarded `platform-check`
  dispatch. If that environment is not prepared, `platform-check` cannot be
  accepted as full-runtime proof; use the established Task-1 aggregate,
  Task-13 coverage, and final-audit responsibility split instead.

## Superseding 2026-09-05 requirement: active aarch64-linux system matrix

The user explicitly adds `aarch64-linux` to flake `supportedSystems`. This is
an authorized t13-s2 remediation, not a sixth sprint: the exact existing DAG
remains `t13-s1 → t13-s2 → t13-s3 → t13-s4 → t13-s5`.

- The current two-system assertion generation is immutable historical evidence:
  its canonical manifest SHA-256 is
  `d3aaca6c53e8e4fa5f5dd6077e1bad92166529579a75a2bb67625faa056c49bb`,
  with its existing manifest, lock, and s1 coordinator record untouched. It
  cannot be reused as the active digest after the expected-system change.
- Before any user card, t13-s2 authoring creates the distinct active generation
  `task13-r12-aarch64-linux-20260905` at
  `tests/nix/task-13-assertion-manifest-aarch64-linux.json`, writes its
  separate `.sha256` lock, and records its computed actual lowercase digest at
  `.agents/results/task-13-s2-aarch64-linux-manifest-digest-20260905.txt`.
  The bare zero-argument `aarch64-linux-manifest` card is zero-mutation
  receipt-only: it derives/prints the coordinator-bound digest and verifies
  new bytes == new lock == new record. It neither writes nor rebaselines any
  active-generation artifact, and rejects the historical d3a digest. Four-way
  caller-digest proof belongs only to `aarch64-linux-shape <recorded-digest>`.
- Test first: before changing `flake.nix`, the bare active-devShell card
  `just --justfile scripts/tasks/task-13/mod.just aarch64-linux-shape
  <recorded-digest>` must report exactly one non-forcing RED,
  `supported-system matrix missing aarch64-linux`, with nested
  `packages`, `devShells`, `checks`, and `supportedSystems` evidence. It must
  not force an aarch64-linux child. After adding the system, the same card must
  be GREEN with exactly `aarch64-darwin,aarch64-linux,x86_64-linux` for
  `supportedSystems` and the three top-level per-system families before child
  inventories. Only then may guarded native `flake-local` run from its one
  immutable filtered snapshot; it cannot claim either Linux realization.
- The two retained Nix check keys keep their exact compile-only commands and
  remain separate. Runtime proof still belongs to Task-1 aggregate, Task-13
  coverage, and final audit; no coverage-floor waiver is introduced.
- Final t13-s5 extends the existing supported-system builder matrix to actual
  `api` and `worker` realization for all three systems. x86_64-linux retains
  the unchanged guarded Task-1 `flake-linux` route. aarch64-linux requires an
  explicit builder/emulation capability and its own descriptor/record evidence;
  absence or failure is a blocker, not a SKIP or substitution. The existing
  FV IDs/order/count, `platform-check`, and 12/3/3 → 15/0/3 → 18/0/0 counts
  remain unchanged. Active r12 changes only FV07's descriptor dispatch array
  to three thin calls in order: x86_64-linux, aarch64-linux, aarch64-darwin.
  Historical r11's two-call array remains byte-preserved. `cross-system-verify`
  never realizes or hides the aarch64-linux build.

### Historical R12 system-matrix evidence and final three-system context

For avoidance of doubt, every earlier R11 phrase in this document that says
“both systems”, “Linux/Darwin”, `ignored-s1-recorded-digest`, an s1 fixed
digest, or a two-descriptor terminal/receipt/handoff is historical only. The
entire R12 system-matrix projection is historical shape evidence only for G2
and authorizes no active card; R14 is the sole active G2/card generation. The
following retained final three-system context does not reopen R12 dispatch:

- The r12 digest/shape establishes retained historical three-system evidence;
  it is not an active G2/t13-s2 dispatch input after G0-F2/R14. Active G2 uses
  only the required R14 order: final plan reviews, authority snapshot copy,
  approval evidence/token, one token-integrity review, four frozen
  sources/guard, manifest/lock/record, final R14 implementation CCR reviews,
  receipt, GREEN-only shape, and guarded-flake-local-r14; old d3a, r12, and
  R13 digests are rejected as active dispatch input.
- t13-s4 keeps 12/3/3 and 15/0/3 counts but evaluates the exact three-system
  flake family shape. It does not realize either Linux package.
- t13-s5 and final records/handoff bind the R14 digest, immutable authority
  snapshots, one GPE map, and all three R14 descriptors. Each descriptor has its own
  reservation, terminal, and first receipt; input drift/failure blocks it.
  The sole active descriptor files are exactly nine generation-specific R14 20260905
  paths: terminal, first receipt, and FV07 replay receipt for each of
  x86_64-linux, aarch64-linux, and aarch64-darwin. No active R14 descriptor
  input may reuse a historical R11/R12/R13 path.
- Active FV01–FV08 input sets use the R14 manifest/record/snapshot/GPE lineage
  and `<r14-recorded-digest>`. Active FV07
  retains its identity/order/count but has exactly three zero-action replay
  descriptor calls: x86_64-linux, aarch64-linux, aarch64-darwin. The
  aarch64-linux descriptor first requires explicit builder/emulation capability
  evidence; unavailable capability blocks rather than skipping.

## Historical 2026-09-05 approval: G0-F1 format-drift successor and R13

The user approved Option A in
`.agents/results/result-pm-task13-g0-format-drift-options-20260905.md`.
This was a controlled G2/t13-s2 successor inside the existing five-task DAG,
not a sixth sprint. It is now immutable historical evidence after G0-F2/R14.
The original G0 completion token and all of its evidence,
R11 manifest/lock/record/frozen sources, and R12 manifest/lock/record/frozen
sources/failure evidence remain immutable historical artifacts. R12 is stale
for active consumption and must never be edited, relocked, or reused.

The sole active successor sequence is:

1. The user runs bare `just --justfile scripts/tasks/task-1/mod.just
   format-check` and returns full PASS raw output.
2. The user runs bare exact existing `just --justfile
   scripts/tasks/task-4b/mod.just redis-publish-config-green` and returns the
   exact eight-test GREEN PASS raw output.
3. A fresh static review confirms the declared two-file formatting-only delta
   and a new versioned G0-F1 record is exclusively created. It includes parent
   and supersedes links to the original G0 token, the approval reference, all
   five allowed-file SHA-256 keys (`docs/commands/task-4b/realtime.md`,
   `scripts/tasks/task-4b/mod.just`, `src/config/mod.rs`,
   `src/config/realtime.rs`, and `src/transport/realtime/composition.rs`), the
   unchanged exact eight-test oracle/selector, format/GREEN/review evidence
   hashes, no-scope-expansion declaration, and freshness timestamp.
4. Only then is R13 authored as new immutable
   `tests/nix/task-13-assertion-manifest-r13-g0-f1.json`, its separate lock,
   and `.agents/results/task-13-s2-r13-g0-f1-manifest-digest-20260905.txt`.
   It freezes exactly `scripts/tasks/task-13/r13-shape.just`,
   `scripts/tasks/task-13/guarded-just-r13.sh`,
   `tests/nix/flake-shape-aarch64-linux-r13.sh`, and
   `scripts/tasks/task-13/flake-source-snapshot-r13.sh`; reused
   `scripts/tasks/task-1/mod.just` is separately bound when delegation occurs.
   Mutable Task-13 `mod.just` remains only outer argv transport.
5. The zero-argument receipt card `just --justfile
   scripts/tasks/task-13/mod.just r13-manifest` writes nothing and only
   derives/echoes the coordinator-bound R13 digest after bytes == lock ==
   record verification. Next `aarch64-linux-shape-r13 <recorded-digest>` is
   GREEN-only for the already-established three-system shape: no R13 RED may
   be manufactured. Only after that GREEN may
   `guarded-flake-local-r13 <recorded-digest>` run through R13's immutable
   snapshot and unchanged Task-1 reuse.

Any failed/missing full format receipt, non-eight-test GREEN, stale review,
missing one of the five hashes, changed oracle/selector, approval/linkage
failure, R13 source/manifest mismatch, or R13 shape non-GREEN blocks with zero
nested dispatch. Historically, R13 carried the existing t13-s2 → t13-s3 →
t13-s4 → t13-s5 path without changing task IDs, FV IDs/order/count, or the
established 12/3/3 → 15/0/3 → 18/0/0 lifecycle. It is not an active G2 card.

## Superseding 2026-09-05 approval: Task-3b contract-drift profile correction

The user approved Option A after R13's guarded native `flake-local` check
reached `checks.aarch64-darwin.contract-drift` and proved that the checker was
comparing two different profiles: a disposable C0 generation tree and the
committed release-candidate tree. This is a narrow active `t13-s2` correction,
not a sixth sprint or a rebaseline. Its original no-R14 boundary is superseded
only by the separately approved append-only G0-F2/R14 immutable-authority
successor below.

- `checks.<system>.contract-drift` continues to invoke
  `scripts/tasks/task-3b/contract-check.sh`; `flake.nix` does not duplicate or
  replace Task-3b provenance, allowlist, generation, or verification logic.
- Generic C0 determinism remains `cargo test --locked --test contract`.
  Legacy generic C0 generation with current extra paths refuses and is not a
  committed-RC check prerequisite; it does not overwrite the committed RC.
  Committed release-candidate verification is exactly `cargo test --locked
  --test production_composition
  contract_generation::allowed_provenance_variants_are_deterministic_and_emit_release_candidate_manifest
  -- --exact`, the existing manifest-aware verify that requires committed RC
  stage, exactly 21 generated artifacts, and exactly 18 owner contribution
  files. The only removed condition is full-tree equality between those two
  profiles.
- Before the exact RC command, the checker uses one shared selector variable
  with the same target/filter and `-- --exact --list`, requiring exactly the
  literal
  `contract_generation::allowed_provenance_variants_are_deterministic_and_emit_release_candidate_manifest: test`.
  Listing failure and zero/multiple marker matches block and propagate; neither
  a pipeline nor command substitution may bypass the gate. The unchanged shared
  selector is then used for the exact RC test.
- This is a checker-only remedy: no `src/contract_generation` edit and no new
  test. The existing exact regression selector must show both retained
  responsibilities and reject the former cross-profile comparison. It must not
  change public contracts, contract artifacts, provenance manifests, fixtures,
  or release-candidate bytes.
- `contract-check.sh` propagates only its native status through `set -e`/normal
  shell semantics and retains the existing cleanup-only
  `trap 'rm -rf -- "$temporary_root"' EXIT` for its exact temporary root. It
  must not add a status-receipt/status-print trap or merge status reporting into
  cleanup. The documented user/operator card owns status evidence: it captures
  the Just status, prints it, then `exit "$status"` returns that same status.
  This avoids duplicate output and does not mask either a failing or successful
  checker.
- R11/R12/R13 manifests, locks, records, frozen sources, receipts, historical
  failures, and R13 digest remain immutable historical evidence. The existing
  five-sprint DAG and all FV IDs/order/count remain unchanged. If the
  correction would require any R13 artifact change, a new checker outside the
  existing manifest-aware path, or loss of either retained verification
  profile, halt and record a blocker rather than rebaselining; the separately
  approved G0-F2/R14 authority successor is the only permitted active-card
  replacement.

## Superseding 2026-09-05 approval: G0-F2/R14 immutable authority successor

R13 correctly rejected the mutable canonical plan/requirements authority after
the approved checker-only amendment changed those planning bytes. The user
explicitly approved Option A: an append-only G0-F2/R14 successor inside the
existing five-task DAG. This is authority renewal only, not a rebaseline. It
does not mutate, relock, replace, delete, relabel, or reactivate original G0,
G0-F1, R11, R12, or R13 evidence.

- At issuance, author immutable versioned snapshots from the final approved
  canonical bytes exactly once at
  `tests/nix/task-13-r14-authority-plan-20260905.json` and
  `tests/nix/task-13-r14-authority-requirements-20260905.md`. Final plan
  reviews precede this copy and their hashes are recorded only in immutable
  pre-token approval evidence
  `.agents/results/task-13-g0-f2-r14-approval-20260905.md`. The append-only
  `.agents/results/task-4b-redis-publish-completion-g0-f2-r14-20260905.json`
  token records only that approval-evidence path/SHA-256 with its final-plan-
  review hashes, each snapshot path/SHA-256, the approval and
  parent/supersedes linkage, retained prerequisite/checker evidence, and
  issuance-time provenance for the mutable canonical source path/SHA-256.
  Exactly one token-integrity review is performed after token issuance and
  before four frozen sources/guard/manifest; its hash may be pinned by the
  new R14 guard/manifest, never by the token. Final R14 implementation CCR
  reviews occur only after manifest/lock/record finalize and are
  coordinator-external pre-card gates, never hashes in token, guard, or
  manifest. Receipt is produced after those final reviews and is external
  output/gate evidence, never a token input. After issuance, the runtime
  R14 guard hashes and consumes snapshots only: it must never read, hash,
  compare, or substitute the mutable canonical paths.
- Author the distinct generation
  `task13-r14-g0-f2-authority-snapshots-20260905` with
  `tests/nix/task-13-assertion-manifest-r14-g0-f2.json`, its separate lock,
  `.agents/results/task-13-s2-r14-g0-f2-manifest-digest-20260905.txt`, and
  exactly four R14 frozen sources:
  `scripts/tasks/task-13/r14-shape.just`,
  `scripts/tasks/task-13/guarded-just-r14.sh`,
  `tests/nix/flake-shape-aarch64-linux-r14.sh`, and
  `scripts/tasks/task-13/flake-source-snapshot-r14.sh`. It may bind unchanged
  `scripts/tasks/task-1/mod.just` only as its delegated target source. R13
  files remain untouched; mutable Task-13 `mod.just` is outer argv transport
  only.
- Snapshot absence/non-regular-file/hash mismatch, issuance provenance
  mismatch, mutable-canonical runtime access, frozen-source/target mismatch,
  token/manifest/lock/record mismatch, failed fresh review, or any attempt to
  bypass the guard fails before nested dispatch. There is no live-current-byte
  fallback and no guard bypass.
- The final order is fixed: final plan reviews -> authority snapshot copy ->
  approval evidence/token -> one token-integrity review -> four frozen
  sources/guard -> manifest/lock/record -> final R14 implementation CCR
  reviews -> receipt -> GREEN-only shape -> guarded flake-local. The
  token-integrity review hash alone may be pinned by R14 guard/manifest. Final
  implementation CCR reviews are coordinator-external pre-card gates and
  receipt is external output; neither may be hashed by token, guard, or
  manifest. Findings block with zero dispatch.
- The only active R14 user cards are bare active-devShell forms:
  `just --justfile scripts/tasks/task-13/mod.just r14-manifest`, then
  `just --justfile scripts/tasks/task-13/mod.just aarch64-linux-shape-r14
  <r14-recorded-digest>`, then
  `just --justfile scripts/tasks/task-13/mod.just guarded-flake-local-r14
  <r14-recorded-digest>`. The receipt is zero-mutation, shape is GREEN-only, and
  guarded native flake-local runs only after final CCR reviews, receipt, and
  shape GREEN. The token-integrity review alone may be guard/manifest-pinned;
  final CCR reviews and receipt are not token/guard/manifest hashes. The documented user/operator card, not the checker, captures the Just
  status, prints it, then `exit "$status"` returns the same status; the checker
  retains normal `set -e` propagation and its existing cleanup-only exact-root
  EXIT trap.
- The approved Task-3b correction remains checker-only: generic C0 generation
  and `cargo test --locked --test contract` determinism remain, while committed
  RC uses the fail-closed exact existing selector and manifest-aware verify.
  G0-F2/R14 adds no new product/API/data contract, test, source-contract
  rewrite, sixth task, FV ID/order/count change, or R13 mutation.

## Historical R11 scope boundary (from `plan-20260822-200110.json` and `session-ultrawork.md`)

- Historical R11 supported systems were exactly `aarch64-darwin` (development)
  and `x86_64-linux` (production); this paragraph is retained historical
  evidence only. The active R14 authority is the authoritative three-system
  matrix above. A missing Linux builder remains an explicit blocker, never a
  silent skip.
- Rootless Podman Compose remains local-development-only disposable
  PostgreSQL/Redis/MinIO. Production is native Nix package/NixOS systemd;
  homelab owns services, hosts, storage, secrets, ingress, monitoring,
  backup, restore.
- Task-13 owns final `flake.nix`, `nix/`, module evaluation, `.env.example`,
  `docs/deployment-nix.md`, and task-owned command scripts only — not
  `README.md` or the root `Justfile`.
- No STT worker/package/config (locked D3=C non-goal).
- No new product decision; D10/D11/D12 (and all other locked decisions) are
  consumed, not reopened.
- VERIFY/SHIP is the sole final cross-task QA audit (PF1 rediscovery, the
  42-operation/2-event legacy matrix, migration-chain/provenance/log/secret/
  license/package evidence, one consolidated whole-tree pass) and runs only
  after Task-13's own ultrawork phases (IMPL/VERIFY/REFINE/SHIP) complete —
  it is not a Task-13 implementation sprint. Final `t13-s5` authors the
  task-owned `final-verify` card/index/recipe contract for its exact ordered
  nine master checks; `final-verify-inventory-boundary` is a static
  not-yet-authored skip before t13-s5 and only passes when those surfaces have
  the frozen IDs/order/references/evidence/blockers, exact wrapper, post-
  SHIP_GATE boundary, and nonempty dispatcher. Its execution remains that
  later cross-task audit, never t13-s5 evidence. The card/recipe binds each
  ordered master check to an exact nonempty dispatch (or typed Ultrawork
  closeout), with direct status propagation and no no-op/status swallowing.
  PF1 and the operation matrix pin `docs/migration-from-jamye-plz.md` SHA-256
  `bdaf1a206f82b7cc64cc388ac1abfdbd1cee2de13572e26b427bd0f8d86f9c3b`,
  the `/Users/poby/Developer/jamye-plz` baseline, and their frozen counts and
  sections before any PASS is possible.

- Historical R11/t13-s1 baseline evidence: before the first Task-13 RED command, `t13-s1` wrote the byte-exact
  SHA-256 lock `tests/nix/task-13-assertion-manifest.sha256` for
  `tests/nix/task-13-assertion-manifest.json` and the first wrapped RED card
  printed and recorded that digest. This retained R11 record is historical-only
  and non-normative after G0-F2/R14; every active t13-s2–t13-s5/FV01–FV09
  card instead validates only the R14 authority snapshots, R14 manifest/lock/
  record and `<r14-recorded-digest>`. Neither historical artifact may be
  edited, relocked, rebaselined, or accepted as active authority.

- Historical R11/t13-s1 evidence also owns `scripts/tasks/task-13/guarded-just.sh`. The bootstrap
  RED card prints the actual 64-hex manifest digest and the coordinator stores
  that literal in `.agents/results/task-13-s1-manifest-digest-20260902-090159.txt`.
  This is retained historical implementation evidence only. Active R14 cards
  take `<r14-recorded-digest>`, compare only R14 snapshot/manifest/lock/record
  authority before any action, and delegate only through the R14 frozen
  adapter. The adapter uses `set -euo pipefail`, `sha256sum`, and `exec just
  --justfile` for direct status propagation; a changed R14 authority tuple is
  not a rebaseline. Task-13 evidence never invokes `just task-1 ...` directly.

- Final FV-04 uses focused guarded targets, not a generic whole-tree result:
  Task-4b `docs/commands/task-4b/realtime.md#redis-publish-config-binding` /
  `scripts/tasks/task-4b/mod.just redis-publish-config-green` supplies the
  completed Redis parser/default/partial-override/invalid-before-side-effect
  binding once, only after the same neutral eight-test inventory and
  byte-identical underlying selector used for Task-4b RED have been evidenced;
  the separate Task-4b `redis-recovery` target supplies
  two-phase delta/Redis recovery once; task-4a supplies PostgreSQL recovery,
  task-8 MinIO resilience and voice `contract-green`, task-9 lease lifecycle
  and privacy mutations, and task-11 deletion/push barriers. Final FV-07 first
  runs Task-13 `linux-builder-check <r14-recorded-digest> <gpe-map-digest>`, a
  thin public target descriptor whose second argument is the exact lowercase
  64-hex GPE map digest returned/echoed by final module-eval and whose Linux
  command is the sole guarded one-time delegate to
  task-1 `flake-linux`. Task-13-owned
  `scripts/tasks/task-13/final-tree-record.sh` is the sole implementation
  owner of source-snapshot validation, descriptor lookup, portable atomic pre-action
  reservation, atomic terminal record creation/hashing, stale-state rejection,
  and first receipt plus the sole FV07 replay-receipt emission. Its record key is exactly
  `(session, assertion_digest, source_snapshot, gpe_artifact_hash_map_sha256,
  target_descriptor_id, descriptor_digest)`. Linux uses descriptor ID
  `x86_64-linux-flake-linux-api-worker`; Darwin uses
  `aarch64-darwin-api-worker`. Each descriptor digest is SHA-256 of canonical
  target ID, exact guarded argv/command, and expected output-map schema. Every
  reservation, immutable terminal record, first receipt, and sole FV07 replay receipt keeps
  both descriptor fields plus the exact ordered four-entry GPE byte-hash map and
  map SHA. The reservation is atomically acquired with same-filesystem
  descriptor-keyed `mkdir` before any
  action; only its holder delegates once, then terminalizes success/failure.
  A concurrent loser performs zero action; interrupted/abandoned reservation,
  terminal failure, or sibling descriptor digest conflict is a hard block with
  no TTL/reclaim/overwrite/deletion/automatic rerun. Only a build_succeeded descriptor permits
  exactly one later FV07-only replay with literal `fv07-replay`, which emits its
  fixed `call_kind=fv07_replay` receipt with count `0`, returns the same terminal exit, and never
  mutates or delegates. A terminal-present call without that mode, wrong mode,
  second FV07 replay, or any later matching call is a zero-action hard block
  with stable non-mutating error and no new receipt. `cross-system-verify`
  is likewise a thin public target descriptor: it never invokes flake-linux or
  rebuilds x86_64-linux api/worker, and its first t13-s5 call realizes the
  final aarch64-darwin api/worker pair exactly once as specified below.

  After all s3-s5 build-input edits and only from the matching successful Linux
  descriptor-scoped terminal record for the same base
  `(session, assertion_digest, source_snapshot, gpe_artifact_hash_map_sha256)`, the first
  `cross-system-verify <r14-recorded-digest>`
  call realizes exactly once
  `packages.aarch64-darwin.api` and `packages.aarch64-darwin.worker`. Its
  immutable descriptor-scoped record preserves `original_invocation_count=1`, literal
  command, exit, unique invocation ID/timestamps, raw-evidence reference,
  both attribute-to-output paths, and `record_sha256`; the first-call receipt
  says count `1`, while the sole exact FV07-only successful-terminal replay receipt
  says `call_kind=fv07_replay`, count `0`, and runs no build. Darwin never treats the Linux record as its own record; it only uses
  it as the prerequisite. Missing/failing/extra output blocks the final matrix and it never
  rebuilds x86_64-linux. Darwin derives the exact map/map SHA from matching
  Linux build_succeeded evidence and re-reads all four paths before its own
  reservation/action; it cannot accept a silently recomputed replacement.

- `final-verify.sh` executes only FV-01 through FV-08 and emits the R14
  source-snapshot-bound `.agents/results/task-13-r14-final-verify-handoff-20260905.json`
  with R14 assertion digest, current complete source snapshot, exact ordered four-entry
  GPE byte-hash map/map SHA (including the fourth artifact), ignored-input
  inventory/coverage-oracle hashes, all three R14 descriptor IDs/digests, all
  three canonical terminal-record hashes, all three first-receipt hashes, all
  three FV07-replay-receipt hashes, ordered per-step input digests plus
  exits/evidence references, and no-mutation declarations. Before every FV-01
  through FV-08 step it calls `final-tree-record.sh` snapshot verification and
  requires the same terminal-recorded four-entry map/map SHA before and after
  each dispatch; a changed fourth file blocks before dispatch rather than
  silently becoming a replacement. It does not reimplement snapshot or reservation
  logic.
  FV-09 is an `external-ultrawork-closeout`, not a shell dispatch: after raw
  user output, the workflow coordinator consumes that handoff and records the
  redacted no-mutation verdict; missing/mismatched handoff or forbidden mutation
  blocks closeout.

## r37 final-verification provenance and evaluator repairs

Same-step FV transcripts are immutable post-dispatch outputs, never FV pre-inputs:
FV03 whole-tree raw result, FV05 migration/ADR audit, FV06 C2 audit, and FV08
dependency/license/log results are canonical-hashed into their own output sets
and the final handoff. FV01 likewise emits a post-dispatch observed legacy
snapshot at .agents/results/task-13-fv01-observed-legacy-snapshot-20260902-090159.nul.
It canonically records NUL-safe selected legacy path/kind/mode/content hashes,
seven document/span byte hashes, HEAD, row count, status entries/fingerprint,
globs, and exclusions before and after audit. PASS requires pre/post equality
and exact equality with the frozen literal selector; handoff retains expected
selector hash, observed snapshot hash, comparison verdict, and raw result hash.
Current-step raw/audit/replay hashes are outputs and handoff fields, never
same-step inputs; only the already-existing Linux/Darwin terminal and first
receipt hashes may be FV07 pre-inputs.
For the session/source key, the final handoff path is absent before FV01 and is
exclusive-created once only after FV01–FV08 accepted outputs/verdicts/hashes.
Any preexisting, malformed, conflicting, or failed creation blocks zero
dispatch; it is never appended, overwritten, reused, or rebaselined. Its
handoff contains no `handoff_sha256`; an external SHA-256 over canonical final
handoff bytes is the sole FV09 input. The closed oracle subkind set
is exactly generated_protocol_evidence, reservation, terminal_record, receipt,
raw_result, audit_result, handoff; unknown/missing/extra values fail.

final-tree-record.sh uses only same-filesystem descriptor-keyed mkdir for
portable atomic process/concurrency/interruption-safe reservation. It makes no
fsync, directory-durability, crash/power-loss, or atomic-rename-durability
claim. An existing reservation directory after interruption is a hard
zero-action blocker without reclaim/delete/overwrite/retry. After each Linux or
Darwin action and before success, it recomputes repository snapshot, ignored
inventory, generated-evidence binding, and descriptor digest. Drift produces an
immutable input_drift terminal/block record with raw action evidence and no
success output map; action failure with drift resolves as input_drift.

Historical R12 module-evaluation wording is retained as non-normative evidence
only and is rejected as active input: one x86_64-linux nixosSystem fixture
proves Linux systemd/MinIO/ordering; separate external lib.evalModules fixtures
for aarch64-darwin, aarch64-linux, and x86_64-linux import
self.nixosModules.default with explicit target pkgs, frozen specialArgs,
minimal option schema, and enablement to prove package defaults and derived
ExecStart provenance. The two non-x86 fixtures make no NixOS/systemd deployment
claim and cannot replace consumer packages; actual api/worker realization is
reserved only for the active R14 three-system t13-s5 descriptor matrix. Every
active t13-s2–t13-s5/FV01–FV09 card validates R14 authority snapshots,
manifest/lock/record/GPE lineage and `<r14-recorded-digest>`; R11/R12/R13 and
ignored-s1 evidence cannot satisfy an active authority check.

For each supported target, the pure provenance fixture uses
`targetPkgs = import nixpkgs { system = target; }` and exact
`specialArgs = { pkgs = targetPkgs; }` only; self remains closure-bound.
Its test-only schema permits only api/worker package paths, assertions, and
systemd.services freeform unit/serviceConfig fields, and rejects any other
emitted parent path. Descriptor terminal, first-receipt, and FV07 replay
receipt are separate exclusive-created immutable files. Every FV output path
is absent before its producer dispatch and exclusive-created after it; conflict
or stale output blocks zero action, while a failed dispatch may record its typed
result once only.
Active R14 handoff names/hashes exactly nine generation-specific R14 20260905 paths:
terminal, first-receipt, and FV07 replay-receipt for each x86_64-linux,
aarch64-linux, and aarch64-darwin descriptor; no JSON fragment is used to
address a mutable shared record. Each adapter `terminal_path` byte-equals its
own canonical R14 terminal map; every first/replay receipt `record_path` equals
that same terminal and its `record_sha256` verifies the canonical parsed terminal
payload, while canonical full-file bytes are separately enforced. Historical R11/R12/R13
six-path/two-map wording is superseded for active R14. Legacy
`task-13-*-build-*.json`, unclassified extra descriptor paths, or a terminal
outside the canonical three-descriptor R14 map are rejected.

Active R14 command-source integrity hashes exactly these four Task-13-owned
immutable sources in its new manifest: `scripts/tasks/task-13/r14-shape.just`,
`scripts/tasks/task-13/guarded-just-r14.sh`,
`tests/nix/flake-shape-aarch64-linux-r14.sh`, and
`scripts/tasks/task-13/flake-source-snapshot-r14.sh`. When guarded-flake-local or
flake-linux delegates, it separately binds `scripts/tasks/task-1/mod.just` as
an additional fifth reused target source, not a replacement for the owned four.
Mutable `scripts/tasks/task-13/mod.just` is outer argv transport only: it is
never reopened as a trusted nested source, so later t13-s3/t13-s4/t13-s5 recipe
authoring does not rebaseline R14. Before snapshot creation, guarded-just-r14
independently revalidates active G0-F2/R14 digest state; before nested
dispatch it verifies each applicable live immutable source and its snapshot copy
against the recorded hash, then dispatches `r14-shape.just` from the immutable
snapshot. Any post-validation live/snapshot mutation, missing copy, or hash
mismatch is a zero-nested-dispatch hard block; a direct live-recipe fallback is
forbidden.

The sole helper rejects duplicate JSON member paths before parse, invalid UTF-8,
wrong top-level fields, and any bytes other than compact lexicographic-key
`jq -cS` JSON plus one LF. `terminal_record_payload` is that canonical object
with only `record_sha256` omitted; `receipt_payload` omits only `receipt_sha256`.
Each displayed digest is derived from its payload and validates that canonical
parsed payload, never a self-hash of final file bytes. Changed payload or
displayed digest, duplicate member, and noncanonical bytes/encoding fail for
both record and receipt; these hashes are immutable protocol outputs, not GPE.

## Verification authority

## Approved prerequisite — Task-4b Redis publish binding

User-approved Option A (2026-09-02,
`decision-request-task13-redis-config-binding-20260902.md`) authorizes, but
does not mark complete, the narrow
`task-4b-redis-publish-config-binding` prerequisite before `t13-s1`. Task-4b
owns a feature-local non-secret parser/config and production realtime worker
composition binding for exactly `JAMYE_REDIS_PUBLISH_LEASE_MS`,
`JAMYE_REDIS_PUBLISH_TIMEOUT_MS`, and
`JAMYE_REDIS_PUBLISH_SAFETY_MARGIN_MS`. Defaults remain exactly 15000ms,
2000ms, and 1000ms. Omission independently selects each default; a partial
override independently replaces only its named value, then the fully resolved
triplet must satisfy positive supported integer milliseconds and
`timeout + safetyMargin < lease`.

Before RED, the only permitted implementation is a behavior-preserving,
default-only compile scaffold: it may privately factor the existing durations
and make tests discoverable, but may not read/bind overrides, reject invalid
values, alter production behavior, or expose a public API. A Task-4b static
scope/compile-surface review (no Nix/Cargo/Just execution) records those facts
before RED; it is not RED/GREEN pass evidence. Private deterministic
`ConfigInput` tests live with `src/config/realtime.rs`; composition tests inject
counting factories into the one private production-compiled helper the public
worker calls, proving invalid resolved input returns before any factory call.

One phase-neutral inventory exists before RED and is executed in this exact
order by both cards: `redis_publish_config_binding::defaults_are_exact_15000_2000_1000_ms`,
`redis_publish_config_binding::all_three_overrides_materialize_outbox_worker_durations`,
`redis_publish_config_binding::partial_override_uses_remaining_defaults`,
`redis_publish_config_binding::invalid_parse_and_bounds_fail_before_side_effects`,
`redis_publish_config_binding::arithmetic_overflow_fails_before_side_effects`,
`redis_publish_config_binding::equality_and_over_budget_fail_before_side_effects`,
`redis_publish_config_binding::key_only_errors_never_echo_raw_values`, and
`redis_publish_config_binding::existing_realtime_worker_default_regression`.
The planned bodies of `redis-publish-config-red` and
`redis-publish-config-green` both dispatch the byte-identical underlying
selector `CARGO_NET_OFFLINE=true cargo test --locked --lib redis_publish_config_binding -- --nocapture`; neither
may mutate or rewrite tests. RED therefore discovers exactly 8 tests and
returns 2 PASS (only `defaults_are_exact_15000_2000_1000_ms` and
`existing_realtime_worker_default_regression`), 6 assertion FAIL (all-three
override, partial override, invalid parse/bounds, defensive synthetic-helper
checked-add overflow,
equality/over-budget, and key-only-error behavior), 0 SKIP, exit 101. Missing
symbols, compilation, DB, Redis, network, service, or harness failures are
invalid RED evidence. GREEN reruns the same inventory/order/filter and returns
exactly 8 PASS/0 FAIL/0 SKIP with exit 0. Every invalid class has zero private
factory counters before error return. The user alone runs the exact cards:
`just --justfile scripts/tasks/task-4b/mod.just redis-publish-config-red` and
`just --justfile scripts/tasks/task-4b/mod.just redis-publish-config-green`.
These prerequisite
cards take no Task-13 `<recorded-digest>` and must not create, require, or infer
one before `t13-s1` materializes it.

Task-13 s1 is blocked until the static scaffold review, exact RED/GREEN raw
evidence, and fresh prerequisite review are returned. This approval amends
planning scope only: implementation/execution remains unauthorized until the
amended PLAN_GATE is reviewed and explicitly confirmed; neither confirmation
flag is set by this approval. Task-13 then exposes the frozen Redis triplet, and module
evaluation proves it maps exactly into the worker environment. It similarly
proves existing Expo and account-deletion triplets map into their owner worker
keys; `.env.example` timing scans are supplemental, not runtime-binding proof.

### r25 enforceable prerequisite and module contracts

G0 is a prerequisite gate, not a sixth Task-13 sprint. Before `t13-s1` writes
its manifest, the coordinator must consume the session-stamped scaffold review,
RED raw output, GREEN raw output, fresh reviewer artifact, and completion token.
That token binds the Option-A approval ID, allowed-file SHA-256 map, lexical
eight-test inventory, exact cards/counts/exits, evidence hashes, reviewer
PASS/hash, no-scope-expansion declaration, and freshness timestamp. The two
Task-4b cards first compare the sorted filtered output of
`CARGO_NET_OFFLINE=true cargo test --locked --lib -- --list` to the eight-name
oracle, then run the identical
`CARGO_NET_OFFLINE=true cargo test --locked --lib redis_publish_config_binding -- --nocapture`
selector. Lease is 1000..=300000ms, publish timeout 100..=30000ms, and safety
margin 1..=30000ms; parse/range validation precedes checked-add and strict
inequality. The full-name lexical oracle is the three
`config::realtime::redis_publish_config_binding::{defaults_are_exact_15000_2000_1000_ms,key_only_errors_never_echo_raw_values,partial_override_uses_remaining_defaults}`
tests followed by the five
`transport::realtime::composition::redis_publish_config_binding::{all_three_overrides_materialize_outbox_worker_durations,arithmetic_overflow_fails_before_side_effects,equality_and_over_budget_fail_before_side_effects,existing_realtime_worker_default_regression,invalid_parse_and_bounds_fail_before_side_effects}`
tests; missing/extra/duplicate names block. Post-range input arithmetic overflow
is unreachable under the caps, so its test injects synthetic near-max durations
into the same private pure budget helper while parse overflow stays in the
parse/bounds test. Key-only errors
identify the offending input key, while checked-add/equality/over-budget use a
stable lease-key budget code and never echo values.

The public realtime `worker` must call one private production-compiled,
factory-parameterized composition helper. Its first operation resolves the
Redis triplet before pool, Redis adapter/client, repository, UUID/owner, or
worker construction; production supplies real factories and colocated tests
inject counters into that exact same helper. The default-only scaffold may
create this seam but cannot bind overrides or reject invalid input before GREEN.

The nine non-secret timing Nix options are authoritative. Both api and worker
services use the exact shared contract
`serviceConfig.EnvironmentFile = [ opaqueEnvironmentFile
generatedTimingEnvironmentFile ]`: the opaque user/SOPS source is first and
must not own timing keys; the generated non-secret file is second, contains
each of the nine timing keys exactly once, is the sole timing-value producer,
and wins on duplicate values under systemd's later-file-wins semantics. Module
evaluation does not open/read/log the opaque file; it proves both ordered lists,
the exact generated nine lines, external duplicate override, and resolved
realtime/push/deletion consumers. `serviceConfig.Environment` contains none of
the nine timing keys for either service.

Before module RED, a static compatibility artifact checks pinned nixpkgs
`56c02bc00adcf003215cc4bd996d6efaf4cff188` at
`nixos/modules/services/web-servers/minio.nix`: all required `services.minio`
options, service `minio`, credentials `EnvironmentFile`, and `minio.service`
dependency. A mismatch is `services.minio interface incompatible`, blocks RED
without altering 12/3/3, and returns alternatives for user decision—never a
custom unit.

The single guarded coverage card uses only nixpkgs `pkgs.cargo-llvm-cov` from
pinned revision `56c02bc00adcf003215cc4bd996d6efaf4cff188`, resolved version
`0.9.0`, with recorded attribute/store-path/version provenance. It derives the
exact lib/bin/integration target oracle in both modes and uses only shared
`CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov`: clean once; locked/offline
default `--workspace --all-targets --no-clean --no-report`; locked/offline
`JAMYE_ENABLE_DEV_FIXTURES=true --workspace --all-targets --all-features
--no-clean --no-report`; then one locked/offline JSON summary-only
`report --fail-under-lines 80`. No clean or replacement occurs between modes;
both retained profiles feed one report. Evidence includes JSON/human summary,
both target inventories, tool/version/store provenance, shared directory, and
retention. `llvm-tools-preview` is the only possible new Rust component after
compatibility evidence; no install or network action is automatic.

The canonical Linux and Darwin immutable terminal records each have exactly three terminal states: `build_succeeded`, `build_failed`, and `input_drift`.
For both Linux and Darwin, successful attribute/output maps
exist only for build_succeeded. build_failed applies only when action fails and
post-action inputs are equal; input_drift preserves raw action evidence and all
post-action comparison fields, has no successful output map, never satisfies
Darwin's Linux-success prerequisite, blocks as failure and is never replay-eligible, and requires a
separately authorized new verification session. If failure and drift coincide,
input_drift takes precedence. It is keyed by
`(session, assertion_digest, source_snapshot, gpe_artifact_hash_map_sha256,
target_descriptor_id, descriptor_digest)` and preserves exactly
one original guarded `flake-linux` delegation, its raw exit/evidence and, on
success, its exact api/worker output map plus `record_sha256`. The t13-s5
first-call receipt records `call_kind=first` and `current_call_invocation_count=1`; only
the sole exact FV07-only successful-terminal replay may emit a separate
`call_kind=fv07_replay` receipt with count `0`, and it does not mutate or delegate.
All other terminal-present calls block without a receipt. Only matching successful Linux terminal state permits the first
Darwin realization; that adapter never invokes flake-linux or rebuilds
x86_64-linux api/worker.

### r29 exported module and final-tree verification identity

`nixosModules.default` is the externally consumable module produced by a module
factory closure bound to this flake `self`. Its exact
`services.jamye-server.api.package` default resolves from
`self.packages.${pkgs.stdenv.hostPlatform.system}.api`, and its exact
`services.jamye-server.worker.package` default resolves from
`self.packages.${pkgs.stdenv.hostPlatform.system}.worker`;
synthetic/repackaged packages and undeclared consumer `specialArgs` are forbidden.
Module evaluation imports `self.nixosModules.default` exactly as an external
consumer and proves both defaults plus native `ExecStart` paths equal the two
existing outputs on each supported system. Any absent, unsupported, or
provenance-mismatched output blocks the existing named assertion lane.

After final 18 PASS/0 FAIL/0 SKIP, module-eval is caller-only: it invokes
`scripts/tasks/task-13/final-tree-record.sh prepare-snapshot <r14-recorded-digest>`
exactly once, then only echoes and binds its returned deterministic NUL-safe
byte-sorted final-tree manifest/hash (separate from s1 assertion digest) over
every tracked and untracked-nonignored repository input, including
missing tracked entries with path/kind/mode/content-SHA-or-symlink-target
representation. It also binds an explicit SHA-256 input catalog/inventory of every ignored
generated/frozen preexisting input consumed by G0 or FV01–FV08: Task-2
PF1/matrix evidence records not already in the repository snapshot, G0
evidence/token, the ignored s1 recorded-digest evidence, and every other ignored
authoritative/reference input. The three Task-2 external selectors are pinned to
`authoritative_inputs.pf1` (legacy root, baseline artifact/SHA, HEAD/row/status
fingerprint and the exact seven-document/two-prompt-span catalog) and
`authoritative_inputs.task2_matrix` (baseline artifact/SHA, three named
sections, and 42 REST/2 realtime counts); repository-owned Task-2 documents are
not duplicated as ignored evidence. The live s1 assertion manifest and lock plus
repository-owned Task-2 documents remain classified only in the repository
snapshot. Only protocol-produced
reservations/terminal records/receipts, per-step raw results, and final handoff
are excluded from base source_snapshot; they are separately hash-linked. Under
the existing three classifier classes, exactly four `immutable_protocol_output`
entries use subkind `generated_protocol_evidence`: the repository manifest,
ignored-input inventory, coverage-oracle artifact, and source-snapshot hash
artifact. In non-self-referential order, the helper canonicalizes repository
entries in memory; derives the closed explicit ignored/external catalog without
generated-evidence content; derives the base G0/FV01–FV08 coverage oracle only
over repository and explicit ignored/external inputs; writes/canonical-hashes
manifest, inventory, and oracle; creates `generated_evidence_binding_payload`
as canonical NUL field-name/value pairs for only `binding_version`,
`tracked_generated_descriptor_ids`, `repository_manifest_sha256`,
`ignored_input_inventory_sha256`, and `input_coverage_oracle_sha256`; derives
`generated_evidence_binding_sha256` from that payload; then derives tagged
NUL-delimited source_snapshot from the component hashes plus that derived digest
and writes the fourth artifact with payload fields followed by the displayed
derived digest. The displayed digest is never payload input; the enclosing
fourth-file byte SHA remains only the separate fourth GPE-map value. The binding
is in-memory/embedded metadata, not a fifth artifact or catalog input. The oracle
never inventories or hashes itself or generated-evidence content. Each catalog
dispatch descriptor has an exact ordered command-string array that is
byte-identical to its `dispatch_by_id` array; its descriptor hash uses that array
without tokenization or inferred commands. Any Task-2 authoritative-input
reference and dispatch JSON-pointer is authoring-time traceability only: runtime
catalog/helper behavior consumes concrete literal Task-2 selector fields or the
tracked dispatch owner/ID/exact command array and hashes, and never dereferences
plan, requirements, or result artifacts.
`scripts/tasks/task-13/final-tree-record.sh prepare-snapshot <r14-recorded-digest>`
is the sole writer/materializer of exactly those four GPE artifacts; neither
module-eval implementation nor `mod.just` writes/canonicalizes snapshot, GPE,
or state. It returns an exact ordered four-entry map of
`{input_id,path,sha256_field,byte_sha256}` in byte-identical artifact order,
`gpe_artifact_hash_map_sha256` as SHA-256 of its canonical NUL tuple stream,
the fourth `source_snapshot_hash_artifact_sha256`, the binding hash, and
`source_snapshot`. The map/digest are in-memory/record metadata only: neither
is a fifth GPE file/catalog entry nor a source_snapshot component. First-run
preparation verifies the recorded digest and that all four
paths are absent, acquires the helper-owned same-filesystem snapshot
reservation, canonicalizes repository and explicit ignored inputs in the
frozen non-self-referential order, prepublish-revalidates mutable inputs, stages
bytes under the reservation, exclusive-creates/publishes the four artifacts in
their frozen order, reopens every path, validates canonical content against
staged bytes, recomputes every byte SHA plus exact map SHA, then
recomputes/revalidates source/ignored/binding before returning the exact map,
map SHA, fourth-artifact hash, component hashes, and source_snapshot. Preexisting, duplicate, conflicting,
malformed, truncated, partial/interrupted, failed-exclusive-create, map-SHA or
read-back mismatch (including the fourth artifact), or drift state is a
zero-Linux/Darwin-action hard block with no overwrite/reuse/rebaseline
and a separately authorized new session; it is not an atomic four-file
transaction and makes no power-loss durability claim. Fixtures cover first
creation success, preexisting/duplicate/conflict zero action, partial/interrupted
blocker, drift during prepare, fourth-artifact postpublish mutation zero action,
and drift after module-eval/prepare before Linux reservation/action.
Historical R11/R12 two-descriptor FV wording and any `ignored-s1-recorded-digest`
input are retained non-normatively only and are rejected as active input. The
active R14 contract names three descriptors—x86_64-linux, aarch64-linux, and
aarch64-darwin—and exactly nine generation-specific terminal, first-receipt,
and FV07-replay-receipt paths. `scripts/tasks/task-13/final-tree-record.sh` is
the only active R14 state-machine owner for all three target descriptors: it
computes/validates R14 authority snapshots and the R14 ignored evidence
inventory, uses descriptor-keyed same-filesystem mkdir as the sole
evidence inventory, uses descriptor-keyed same-filesystem mkdir as the sole
portable atomic reservation before any action, atomically terminalizes/hashes immutable records, rejects stale/
malformed/duplicate/nonterminal/conflicting state, and emits only the first
receipt plus the sole FV07 replay receipt for each R14 descriptor. Blocked calls
return a stable non-mutating error and create no receipt. The x86_64-linux
descriptor supplies only the guarded task-1 delegate; the aarch64-linux
descriptor supplies only its explicit capability-bound realization; the Darwin
descriptor supplies only the locked two-attribute build and requires matching
R14 Linux descriptor-scoped terminal success. `final-verify.sh` never owns this
state machine; before and after every FV01–FV08 step it calls this helper's R14
snapshot verification and consumes its exact ordered R14 FV input set: the
complete expanded repository-ID array/component hash, concrete ignored/external
IDs and hashes (including FV01 legacy-root selectors but excluding ignored-s1),
generated-evidence IDs/hashes resolved only from the same R14 terminal-recorded
four-entry map/map SHA, dispatch descriptor, and preexisting R14 protocol
IDs/hashes. FV07 pre-step inputs are the three R14 terminal records and three
first receipts; its replay receipts are outputs, never pre-step inputs. It
requires equality for each pre/post component and records canonical step digest plus raw
result hash. The digest is SHA-256 of NUL-delimited FV ID and ordered tuples
`(input_id, classification/subkind, path-or-selector, sha256)`; no inferred
input is permitted. FV07 invokes exactly three public descriptors in order with
zero build: `linux-builder-check <r14-recorded-digest> <gpe-map-digest>
fv07-replay`, `aarch64-linux-builder-check <r14-recorded-digest>
<gpe-map-digest> fv07-replay`, then `cross-system-verify
<r14-recorded-digest> fv07-replay`; it hands their R14 paths, hashes, and replay
receipts to the R14 FV09 handoff. A changed final tree needs separately
authorized verification.

- Test approach: primarily `test_after` at the milestone level (per the
  master plan's task-13 record: `test_scope=[integration, package,
  module-evaluation]`, `tdd_evidence_required=false`), because Nix package/
  check/module correctness is ultimately proven by supported-system builds
  and evaluation, not by unit-level red/green cycles.
- Within that milestone, this sub-plan applies `tdd`-style red/green
  discipline wherever a meaningful behavioral distinction exists before
  implementation (static contract-shape assertions, module-evaluation
  option/negative-path assertions, coverage-threshold gate), and reserves
  `test_after` only for pure package-build sprints where "RED" would
  otherwise be a bare Nix evaluation crash — which this plan explicitly
  treats as invalid evidence, mirroring the Task-11/Task-12 precedent that a
  compile/evaluation crash is never valid RED.
- The agent authors static contract tests, module-evaluation assertions,
  Nix/flake/module source, `.env.example` additions, documentation, and exact
  command cards. The user alone runs every Nix/Cargo/build/test/fmt/Clippy/
  coverage/module-evaluation command and returns raw output.
- No commit, push, deployment, production access, or remote mutation is
  authorized by this request.

## Existing approved alternatives (process-level, this session)

- A single non-sequential Task-13 sprint covering package/module/coverage/
  docs together was rejected: it would produce an unreviewable RED/GREEN
  batch and obscure which of the six acceptance-criteria responsibility groups
  is satisfied by which evidence. The current review-driven plan uses exactly
  five sequential sprints (`t13-s1` through `t13-s5`): final environment
  additions, deployment documentation, command cards/index, Linux-gated
  recipes, and cross-system documentation were merged into final `t13-s5`.
  Their non-parallel dependency chain is exact: `t13-s1 → t13-s2 → t13-s3 →
  t13-s4 → t13-s5`; G0 Task-4b completion remains the authorized prerequisite
  before `t13-s1`, not a sixth sprint.
  That merger preserves evidence: it authors those final surfaces first, then
  runs one unchanged complete-surface module-eval with exactly 18 PASS/0
  FAIL/0 SKIP, then `linux-builder-check`, then conditional
  `cross-system-verify`. RED is used only where a meaningful behavioral
  distinction exists; the merger does not remove an evidence responsibility.
- Running NixOS-module verification as a full `nixos-test` VM boot test was
  rejected in favor of static `evalModules`/`nixosSystem` dry evaluation: VM
  tests require Linux/KVM not provided by the `aarch64-darwin` development
  machine and are not reproducible without an unpinned hypervisor dependency;
  the master criteria's own wording ("module evaluation ... proves") matches
  static evaluation, not runtime service behavior.
- `cargo-tarpaulin` was considered and rejected in favor of `cargo-llvm-cov`
  for the coverage gate: tarpaulin's ptrace-based instrumentation is
  unreliable outside Linux and does not support `aarch64-darwin` development
  builds, while `cargo-llvm-cov` uses LLVM source-based coverage already
  compatible with the existing `fenix`/`rustc` toolchain on both supported
  systems.
- Hand-rolling a bespoke systemd unit for local MinIO was rejected in favor
  of configuring upstream NixOS `services.minio` directly: it already owns
  user/group/`dataDir`/service wiring, reducing Task-13's module to the
  loopback-console, `rootCredentialsFile`, and non-blocking-ordering wiring
  actually required by the locked criteria, without duplicating logic nixpkgs
  already maintains and tests upstream.

No new product decision is opened beyond the already-locked D10/D11/D12
consumption recorded above.

## Superseding 2026-09-05 approval: append-only R15 coverage-authority successor

User-approved Option A resolves the R14 coverage-authority contradiction found
by the architecture review. R14's immutable guard correctly rejects both
coverage actions because neither public pair exists in its exact 3 immutable +
15 live allowlist. R14 therefore remains historical-only and is not patched,
relocked, reused, or treated as a continuing receipt/capability. R15 is the
sole active authority for `t13-s2`–`t13-s5` and `FV01`–`FV09`; R11–R14 and all
of their manifests, locks, records, snapshots, tokens, reviews, sources,
receipts, digests, and evidence remain byte-for-byte historical artifacts.

### R15 issuance and immutable authority contract

No R15 artifact may be issued until final R15 plan-completeness, plan-meta, and
plan-simplicity reviews have each returned PASS against the final canonical
plan/requirements bytes. Only then, and in this non-self-referential order,
author:

1. Immutable copies `tests/nix/task-13-r15-authority-plan-20260905.json` and
   `tests/nix/task-13-r15-authority-requirements-20260905.md` from those final
   canonical bytes.
2. Pre-token approval evidence
   `.agents/results/task-13-g0-f3-r15-approval-20260905.md` and successor
   token `.agents/results/task-4b-redis-publish-completion-g0-f3-r15-20260905.json`.
   The token may bind parent/supersedes, approval path/hash, R15 snapshot
   paths/hashes, retained prerequisite/checker provenance, and issuance-only
   canonical-source path/hash. It must not bind its own hash, a later review,
   frozen sources, manifest, lock, record, CCR review, receipt, RED/GREEN, or
   FV output.
3. Exactly one post-token integrity PASS review at
   `.agents/results/review-task13-g0-f3-r15-token-integrity-r1-20260905.md`.
   This is the sole review hash an R15 guard/manifest may pin.
4. The four frozen R15 sources
   `scripts/tasks/task-13/r15-shape.just`,
   `scripts/tasks/task-13/guarded-just-r15.sh`,
   `tests/nix/flake-shape-aarch64-linux-r15.sh`, and
   `scripts/tasks/task-13/flake-source-snapshot-r15.sh`; then
   `tests/nix/task-13-assertion-manifest-r15-g0-f3.json`, its `.sha256` lock,
   and `.agents/results/task-13-s2-r15-g0-f3-manifest-digest-20260905.txt`.
5. Final R15 implementation CCR reviews as external pre-card gates, then
   R15 receipt, GREEN-only shape, and guarded-flake-local. These final reviews
   and receipt are outputs/gates, never token/guard/manifest hash inputs.

At runtime `guarded-just-r15.sh` validates and consumes only the R15
snapshots/manifest/lock/record/token chain/frozen-source hashes. It must never
open, hash, compare, substitute, or fall back to mutable canonical
`.agents/results/plan-20260902-090159.json` or requirements paths. Any
missing/nonregular/symlink/hash/order/class/target mismatch fails before nested
dispatch, with no receipt-only or direct mutable-recipe bypass.

### Coverage dispatch, RED staging, and TOCTOU

The R15 manifest/guard partition must be exactly `3 immutable_snapshot + 17
live_worktree = 20` unique pairs, with exact equality between `allowed_pairs`
and the one-and-only-one `execution_classes` classification. The only two
added live-worktree pairs are private Task-13 executors:

- `scripts/tasks/task-13/mod.just::coverage-red-r15-exec`
- `scripts/tasks/task-13/mod.just::coverage-r15-exec`

The public cards remain bare-devShell argv transports only:

- `just --justfile scripts/tasks/task-13/mod.just coverage-red <r15-recorded-digest>`
- `just --justfile scripts/tasks/task-13/mod.just coverage <r15-recorded-digest>`

They invoke the frozen R15 guard and are never nested target pairs. The guard
copies the selected private executor Justfile adjacent to the live target,
revalidates device/inode/content immediately before dispatch, and invokes only
that copy with exported `TASK13_ASSERTION_DIGEST`. The executor does not trust
an independent caller digest. A deterministic fixture must mutate or
path-replace the selected coverage executor after copy; the result is exit 2
and zero nested dispatch. This closes the receipt-then-mutable-executor TOCTOU
gap.

Before the user RED run, define only public `coverage-red`, private
`coverage-red-r15-exec`, and `tests/nix/coverage-red.sh`. The script checks
that both the public/private GREEN recipes are absent and `cargo-llvm-cov` is
unavailable, reporting exactly `coverage recipe missing` and
`cargo-llvm-cov devShell tool missing`: `2 FAIL / 0 PASS / 0 SKIP`, non-zero.
Do not predefine public `coverage`, private `coverage-r15-exec`,
`pkgs.cargo-llvm-cov`, or `llvm-tools-preview`. Only after retained user RED
evidence may GREEN add those two recipes plus the approved pinned Nix tool and
toolchain component as necessary; the existing all-target/all-feature,
two-mode, and >=80% coverage contract remains unchanged.

### Active lineage, invariants, and rejected alternatives

`t13-s2` receipt/shape/guarded-flake-local, `t13-s3` coverage, `t13-s4`
module-eval, `t13-s5` descriptors/final-tree records/handoff, and FV01–FV09
all consume R15 lineage. FV03 retains its ID, position, count, and fifth
coverage slot exactly; it calls the R15 public coverage transport, which then
selects the private R15 executor. FV IDs/order/count and the existing
five-sprint dependencies remain unchanged:
`t13-s1(task-12, task-4b-redis-publish-config-binding) → t13-s2 → t13-s3 →
t13-s4 → t13-s5`.

- R14 receipt followed by direct coverage was rejected: a receipt cannot bind
  later mutable executor bytes and creates a TOCTOU/direct-bypass path.
- Reusing/smuggling an existing R14 pair or nesting public coverage names was
  rejected: it lacks coverage authority, recurses, or illegally mutates R14.
- Append-only R15 with two distinct private executor pairs was approved: it
  retains audit history while authorizing the required cards fail-closed.

This is planning only. It authorizes neither R15 issuance nor coverage,
tooling, product-code implementation, or any executable validation before the
new plan reviews have passed and the user separately authorizes execution.

### R15 active-projection correction

The full R15 protocol projection, rather than any retained R14 detailed card,
is normative for implementation. Every former active R14 reference in task
scope, acceptance criteria, user cards, expected evidence, blockers, G2–G5,
TDD sequencing, final-audit wording, assertion manifests, GPE/final-tree
records, descriptors, receipts, handoff, and FV01–FV09 is historical prose
only. It is not an implicit compatibility alias and cannot be selected by a
receipt, digest, fallback, or current-byte substitution.

The R15 projection owns the byte-identical inherited flake/module/timing/
deployment/coverage behavioral values, including the coverage four-command
array, as values of `assertion_manifests.task13_r15`. It does not look up R14
at runtime to reproduce them. R15's explicit 20-pair partition is the three
immutable pairs (R15 shape, Task-1 flake-local, Task-1 flake-linux), the 15
inherited live focused pairs, and only the two new private live executor pairs
`coverage-red-r15-exec` and `coverage-r15-exec`; public coverage cards remain
transports and are not allowlisted nested pairs.

All active final-protocol paths use `<r15-recorded-digest>` and R15-only
snapshots/token-review/manifest/lock/record/frozen sources. The three
descriptor families use new R15 terminal, first-receipt, and FV07 replay-
receipt files, and FV09 consumes only
`.agents/results/task-13-r15-final-verify-handoff-20260905.json`. The exact
counts (12/3/3, 15/0/3, 18/0/0), five-sprint DAG, FV01–FV09 IDs/order/count,
and FV03 fifth public coverage transport are unchanged.

The sole coverage assertion is `assertion_manifests.task13_r15.coverage`.
Its RED public transport selects only `coverage-red-r15-exec` and reports the
two exact failures with 2/0/0/non-zero; its GREEN public transport selects
only `coverage-r15-exec` and reports 1/0/0/0. R15 owns the byte-identical
four-command array, pinned `pkgs.cargo-llvm-cov` 0.9.0 provenance, Rust
1.98.0 plus `llvm-tools-preview` preflight, shared target directory, two
inventories, and one >=80% report. No R14 coverage dereference is a runtime
or hard assertion dependency.

## Approved 2026-09-06 R16 successor amendment: cargo-llvm-cov 0.9.0 CLI compatibility

This final section has precedence over every earlier statement that calls R15
active. R11 through R15, their manifests, locks, records, snapshots, tokens,
reviews, frozen sources, receipts, and runtime evidence are immutable historical
evidence. The sole active Task-13 authority is the append-only R16 generation
`task13-r16-g0-f4-coverage-cli-compat-20260906` after its complete issuance
chain succeeds. No R15 byte may be edited, relocked, reinterpreted, or accepted
as an R16 runtime input.

The user approved Option A after the pinned `cargo-llvm-cov 0.9.0` rejected the
R15 collection argv with `--no-report may not be used together with
--no-clean`. The accepted correction removes only `--no-clean` from collection
commands two and three. `--no-report` remains on both commands and is the
pinned tool's artifact-retention mode. Every other coverage invariant remains
unchanged:

1. `CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov clean --workspace`
2. `CARGO_NET_OFFLINE=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov --locked --offline --workspace --all-targets --no-report`
3. `CARGO_NET_OFFLINE=true JAMYE_ENABLE_DEV_FIXTURES=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov --locked --offline --workspace --all-targets --all-features --no-report`
4. `CARGO_NET_OFFLINE=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov report --locked --offline --json --summary-only --fail-under-lines 80`

Each fresh R16 execution starts with command one and therefore never reuses the
clean-only state left by the failed R15 run. Commands two and three use the
same target directory, remain locked/offline and all-target, and have no clean,
target replacement, or report between them. The executor proves that default
profiles survive the fixture/all-features collection and that the second mode
adds profiles before command four emits the sole report. Tool substitution,
version drift, missing Rust 1.98.0 or `llvm-tools-preview`, target-inventory
drift, a missing mode, an intervening clean, replaced default profiles, no
profile growth, a second report, or line coverage below 80 percent blocks.

R16 preserves the pinned `pkgs.cargo-llvm-cov` attribute, nixpkgs revision
`56c02bc00adcf003215cc4bd996d6efaf4cff188`, resolved version `0.9.0`, Rust
1.98.0, `llvm-tools-preview`, both frozen target inventories, the shared
`target/task-13-llvm-cov` directory, JSON and human evidence, exact summary
`1 PASS / 0 FAIL / 0 SKIP`, and exit 0. It adds no dependency pin change,
product/API/data-contract change, migration, task, sprint, FV ID, or test.

R15 coverage RED remains the accepted historical staged-RED authorization:
exactly `2 FAIL / 0 PASS / 0 SKIP`, non-zero, with `coverage recipe missing`
and `cargo-llvm-cov devShell tool missing`. It is not rerun under R16. The
three failed R15 GREEN attempts remain historical failure evidence and cannot
satisfy R16 GREEN.

The only active coverage public surface is
`just --justfile scripts/tasks/task-13/mod.just coverage-r16
<r16-recorded-digest>`. It is a thin argv transport to
`guarded-just-r16.sh`, which may select only the private live-worktree recipe
`coverage-r16-exec` from an adjacent read-only copy after final
device/inode/SHA-256 validation. Public R15 `coverage` and `coverage-red` and
their private executors are historical and absent from the R16 allowlist. The
R16 allowlist partition is exactly three immutable-snapshot pairs plus sixteen
live-worktree pairs, total nineteen. A new R16 deterministic mutation fixture
targets `coverage-r16-exec`; both content mutation and path replacement must
exit 2 with zero nested dispatch and zero persistent repository mutation.

R16 issuance order is fixed: these plan and requirements snapshots; the three
fresh current-byte plan reviews; immutable approval evidence binding the user
decision and those review hashes; append-only G0-F4 token whose parent is R15;
exactly one fresh token-integrity PASS review; four R16 frozen sources and the
R16 mutation fixture; canonical R16 manifest, lock, and coordinator digest
record; three fresh implementation CCR reviews; user-owned zero-mutation R16
receipt; user-owned R16 GREEN-only three-system shape; user-owned R16 guarded
flake-local; then user-owned R16 coverage GREEN. A later card cannot satisfy an
earlier gate, and R15 receipt, shape, flake, CCR, or GREEN-failure evidence does
not satisfy the corresponding R16 gate.

All user cards are bare `just --justfile ...` invocations inside the already
active Nix devShell, with status capture, printing, and rethrow. The agent does
not run Nix, Cargo, Just, build, test, format, Clippy, coverage, module, package,
Git, SCM, deployment, or production commands. FV03 retains its identity, order,
and fifth coverage slot but that slot becomes the R16 public coverage transport.
All later module, descriptor, GPE, final-tree, handoff, and FV01-FV09 authority
uses only R16 snapshots/token/review/manifest/lock/record/frozen sources and
`<r16-recorded-digest>`.

## Approved 2026-09-06 R17 successor amendment: coverage test-harness serialization

This R17 file is the complete byte-for-byte R16 requirements file above,
followed only by this append-only amendment. The R16 bytes are retained rather
than summarized, reinterpreted, or replaced. This amendment has precedence
over every earlier statement that marks R16 active only after the complete R17
issuance chain succeeds. Until then, R17 is a candidate and has no active
runtime authority.

After issuance, the sole active Task-13 authority is R17 generation
`task13-r17-g0-f5-coverage-test-serialization-20260906`. R11-R16—including
the complete inherited R16 snapshot and its embedded R11/R14/R15/R16 plan
objects—remain byte-preserved historical artifacts. No earlier plan,
requirements, approval, token, review, frozen source, fixture, manifest,
lock, record, receipt, digest, executor, profile, report, descriptor, or
handoff may be altered, relocked, reused, or accepted as active R17 input.

R16's receipt, exact three-system shape, and guarded local flake cards passed.
Its user-owned coverage card passed authority/tool/LLVM/inventory preflight,
cleaned once, and then ended exit 101 in default-mode `tests/media.rs`: 83
tests passed and exactly four message-binding tests returned safe
`Error: Unavailable`. Fixture/all-feature collection and the sole report did
not run. That result at
`.agents/results/task-13-s3-r16-coverage-green-media-unavailable-user-evidence-20260906.md`
is historical blocked evidence, never GREEN. It cannot be blindly retried or
used to reuse partial profiles.

R17 is not a sixth sprint, a new RED stage, or a product/API/data/migration
change. It does not modify `tests/support/postgres.rs`, production adapter or
logging behavior, Compose/PostgreSQL `max_connections`, target inventory,
coverage threshold, Rust/cargo-llvm-cov/nixpkgs/LLVM pins, or any test
assertion. The static diagnosis treats coverage-instrumented disposable
PostgreSQL fixture pressure only as a leading hypothesis; R17 preserves the
tests and their failure visibility instead of claiming a confirmed SQLSTATE.

### Exact R17 coverage sequence

Commands one and four are byte-identical to R16. Commands two and three differ
only by appending the Rust test-harness passthrough ` -- --test-threads=1`:

1. `CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov clean --workspace`
2. `CARGO_NET_OFFLINE=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov --locked --offline --workspace --all-targets --no-report -- --test-threads=1`
3. `CARGO_NET_OFFLINE=true JAMYE_ENABLE_DEV_FIXTURES=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov --locked --offline --workspace --all-targets --all-features --no-report -- --test-threads=1`
4. `CARGO_NET_OFFLINE=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov report --locked --offline --json --summary-only --fail-under-lines 80`

The suffix is after cargo-llvm-cov's `--` separator and serializes only
between-test scheduling in each collected harness. It removes no target or
test and does not change concurrency deliberately exercised inside a test.
Every R17 attempt starts with command one exactly once. Both collection modes
remain locked/offline, workspace-wide, all-target, shared-directory,
`--no-report` commands. No clean, replacement, or report may intervene;
default raw profiles must survive and combined raw profiles must strictly grow
before the sole JSON report. The existing 80-percent line floor, both frozen
inventories, pinned `pkgs.cargo-llvm-cov` 0.9.0/nixpkgs revision
`56c02bc00adcf003215cc4bd996d6efaf4cff188`, Rust 1.98.0, and
`llvm-tools-preview` remain exact.

### R17 issuance, dispatch, and later lineage

The candidate snapshots are
`tests/nix/task-13-r17-authority-plan-20260906.json` and this file. Before
their R16-parented approval/token chain, fresh exact-byte plan-completeness,
plan-meta, and plan-simplicity reviews must PASS. The ensuing approval,
token, sole post-token review, four frozen sources, R17 TOCTOU fixture,
manifest, lock, and record use these exact R17 paths:

- `.agents/results/task-13-g0-f5-r17-approval-20260906.md`
- `.agents/results/task-4b-redis-publish-completion-g0-f5-r17-20260906.json`
- `.agents/results/review-task13-g0-f5-r17-token-integrity-r1-20260906.md`
- `scripts/tasks/task-13/r17-shape.just`
- `scripts/tasks/task-13/guarded-just-r17.sh`
- `tests/nix/flake-shape-aarch64-linux-r17.sh`
- `scripts/tasks/task-13/flake-source-snapshot-r17.sh`
- `tests/nix/coverage-executor-toctou-r17.sh`
- `tests/nix/task-13-assertion-manifest-r17-g0-f5.json`
- `tests/nix/task-13-assertion-manifest-r17-g0-f5.sha256`
- `.agents/results/task-13-s2-r17-g0-f5-manifest-digest-20260906.txt`

The token binds only pre-issuance R16-parent, approval, snapshot, and retained
provenance facts. It cannot bind itself, any future hash, its integrity review,
frozen sources, fixture, manifest, lock, record, CCR reviews, runtime output,
coverage result, or FV output. The R17 TOCTOU fixture must separately exercise
content mutation and path replacement of `coverage-r17-exec`, each exiting 2
with zero nested dispatch and zero persistent repository writes.

The R17 pair partition is exactly `3 immutable_snapshot + 16 live_worktree =
19` pairs. `coverage-r17-exec` is the only Task-13 coverage nested pair; R15
and R16 executors and every public transport are excluded. The public card is
only `just --justfile scripts/tasks/task-13/mod.just coverage-r17
<r17-recorded-digest>`, a thin transport through the R17 guard to a verified
read-only adjacent copy of the private executor. Direct, recursive,
receipt-only, R16-fallback, and partial-profile paths block.

After final R17 implementation CCR reviews, the user, already in the active
devShell, runs only this ordered R17 lineage:

1. `just --justfile scripts/tasks/task-13/mod.just r17-manifest`
2. `just --justfile scripts/tasks/task-13/mod.just aarch64-linux-shape-r17 <r17-recorded-digest>`
3. `just --justfile scripts/tasks/task-13/mod.just guarded-flake-local-r17 <r17-recorded-digest>`
4. `just --justfile scripts/tasks/task-13/mod.just coverage-r17 <r17-recorded-digest>`

No outer `nix develop` wrapper is valid evidence. R17 coverage must produce
exactly `1 PASS / 0 FAIL / 0 SKIP`, exit 0, one JSON report, provenance and
retention evidence, and at least 80-percent line coverage. Any R17 failure is
blocked evidence and never allows profile reuse. `t13-s4`, `t13-s5`, GPE,
descriptors, handoff, and FV01–FV09 all rebind to R17 after R17 coverage GREEN;
FV03 retains its existing ID/order/count/fifth coverage slot, while the module
counts and three-system materialization matrix remain unchanged.
# R18 / G0-F6 strict review-binding correction amendment — 2026-09-06

## Authority and preservation

Generation: `task13-r18-g0-f6-review-binding-correction-20260906`.

The user approved strict Option A on 2026-09-06. R18 is an append-only
successor to R17. R11 through R17, including every plan, requirements
snapshot, review, approval, token, source, fixture, manifest, lock, record,
receipt, log, digest, and defect report, remain byte-preserved historical
artifacts. R18 must never edit, relock, reinterpret, or dispatch R17.

The sole authority correction is historical R17 completeness-review binding:

- R17 plan nested path (failed):
  `.agents/results/review-task13-g0-f5-r17-plan-completeness-r1-20260906.md`
  — `FAIL`, SHA-256
  `875e9f1629cce9a02581070eb93baf5cf0c0869e5f0a93b69284982719e4519b`.
- R17 approval/token binding (passing):
  `.agents/results/review-task13-g0-f5-r17-plan-completeness-r2-20260906.md`
  — `PASS`, SHA-256
  `034c34a46756b2f48ddd2872f1059e2b6ff4715b987d68e344b8f568c4cb0479`.
- Confirmed defect evidence:
  `.agents/results/bugs/bug-20260906-r17-plan-review-revision-mismatch.md`.

R18 corrects that mismatch only by requiring fresh R18 exact-byte plan reviews
and binding their final shared review set consistently in the R18 plan,
approval, and R17-parented R18 token. R17 remains historical blocked
authority; R18 does not repair R17 in place.

## Immutable snapshots and issuance paths

- Plan snapshot:
  `tests/nix/task-13-r18-authority-plan-20260906.json`
- Requirements snapshot:
  `tests/nix/task-13-r18-authority-requirements-20260906.md`
- Approval:
  `.agents/results/task-13-g0-f6-r18-approval-20260906.md`
- Parent token:
  `.agents/results/task-4b-redis-publish-completion-g0-f5-r17-20260906.json`
- R18 token:
  `.agents/results/task-4b-redis-publish-completion-g0-f6-r18-20260906.json`
- Sole post-token review:
  `.agents/results/review-task13-g0-f6-r18-token-integrity-r1-20260906.md`
- Frozen sources:
  `scripts/tasks/task-13/r18-shape.just`,
  `scripts/tasks/task-13/guarded-just-r18.sh`,
  `tests/nix/flake-shape-aarch64-linux-r18.sh`, and
  `scripts/tasks/task-13/flake-source-snapshot-r18.sh`
- TOCTOU fixture: `tests/nix/coverage-executor-toctou-r18.sh`
- Manifest and lock:
  `tests/nix/task-13-assertion-manifest-r18-g0-f6.json` and
  `tests/nix/task-13-assertion-manifest-r18-g0-f6.sha256`
- Coordinator record:
  `.agents/results/task-13-s2-r18-g0-f6-manifest-digest-20260906.txt`

The R18 token may bind only pre-issuance facts: its R17 parent token, confirmed
R17 defect evidence, R18 approval, R18 snapshot path/hash pairs, and the three
final R18 plan review path/hash pairs. It must not self-hash or bind a future
post-token review, frozen source, fixture, manifest, lock, record, CCR review,
runtime receipt, coverage output, or FV output.

## Mandatory review-set cross-consistency gate

The final R18 candidate plan must name exactly these three fresh review paths:

1. `.agents/results/review-task13-g0-f6-r18-plan-completeness-r1-20260906.md`
2. `.agents/results/review-task13-g0-f6-r18-plan-meta-r1-20260906.md`
3. `.agents/results/review-task13-g0-f6-r18-plan-simplicity-r1-20260906.md`

Before approval or token issuance, every listed review must exist, state PASS,
and hash-match its plan/approval/token projection. The final candidate plan's
ordered review path set must equal exactly the ordered review path set bound by
both approval and token. A failed or superseded revision requires updating the
still-candidate R18 plan and restarting all three exact-byte reviews. The sole
post-token review must independently re-check the nested-plan-versus-
approval/token equality. No review revision can be silently selected by an
approval, token, guard, manifest, or runtime transport.

## R18 runtime contract

R18 runtime uses only its own immutable snapshots, R18 token, sole post-token
review, manifest, lock, record, frozen source hashes, and TOCTOU fixture.
Mutable canonical plan/requirements and every R11-R17 artifact are forbidden
runtime inputs and fallbacks. Parent R17 token and defect evidence are token
provenance only; the R18 guard, manifest, and executor must never open, hash,
compare, or dispatch R11-R17 artifacts.

The R18 assertion is self-contained at runtime: exactly 19 unique allowed
pairs, partitioned as 3 immutable-snapshot and 16 live-worktree pairs. The
only Task-13 private coverage executor is
`scripts/tasks/task-13/mod.just::coverage-r18-exec`. The public user cards are
`r18-manifest`, `aarch64-linux-shape-r18`, `guarded-flake-local-r18`, and
`coverage-r18`; `coverage-r18-exec` is private. Every R11-R17 executor and
every public transport is excluded.

R18 rebinds the complete inherited operator, deployment/timing, final-tree,
descriptor, GPE, handoff, and FV01-FV09 lineage to R18 paths and digest. The
five Task-13 sprint IDs/dependencies, target inventory, tooling/pins, coverage
floor, module/package scope, no-new-RED rule, and all other inherited behavior
remain unchanged.

## Coverage argv invariance

All four R18 coverage commands are byte-identical to R17. Commands two and
three retain their sole terminal ` -- --test-threads=1`; commands one and four
remain unchanged. R18 makes no product, test, DB, Compose, toolchain, pin,
inventory, threshold, API, migration, or deployment change.

1. `CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov clean --workspace`
2. `CARGO_NET_OFFLINE=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov --locked --offline --workspace --all-targets --no-report -- --test-threads=1`
3. `CARGO_NET_OFFLINE=true JAMYE_ENABLE_DEV_FIXTURES=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov --locked --offline --workspace --all-targets --all-features --no-report -- --test-threads=1`
4. `CARGO_NET_OFFLINE=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov report --locked --offline --json --summary-only --fail-under-lines 80`

Candidate planning and later static authority authoring are authorized. No
Nix, Cargo, Just, build, test, format, Clippy, coverage, module, package, SCM,
network, service, deployment, production, or product-source operation is
authorized by this amendment.

## Append-only 2026-09-06 R19 amendment: guard argument-generation consistency

R18 is immutable blocked historical authority. Its plan assertion generation is
`task13-r18-g0-f6-review-binding-correction-20260906`, while the first token
of that assertion's `guarded_dispatch.arguments` is the different
`task13-r18-g0-f6-coverage-test-serialization-20260906`. Its plan,
requirements, approval, token, sole post-token review, confirmed defect
report, and five partially authored runtime source/fixture files remain
byte-preserved historical evidence. R18 has no manifest, lock, coordinator
record, receipt, or runtime GREEN claim; none may be invented, relocked,
reinterpreted, or dispatched.

The strict Option A successor is the distinct candidate generation
`task13-r19-g0-f7-guard-argument-consistency-20260906`. R19 corrects only
that R18 identity mismatch. Its assertion generation, successor token
generation, first `guarded_dispatch.arguments` token, public guard literal,
and exported guard environment generation must all equal that exact string.
The equality is mandatory before approval/token issuance, in the sole
post-token integrity review, and in the R19 guard and manifest. Any mismatch
blocks before receipt or nested dispatch with zero nested action.

R19 requirements preserve all four R18/R17 coverage command argv strings
byte-for-byte. Commands two and three retain their existing terminal
` -- --test-threads=1`; commands one and four are unchanged. The historical
R16 coverage failure remains blocked evidence and is rerun only as one fresh
R19 sequence (`rerun_under_r19: fresh_R19_sequence_only`), never by reuse of
R15/R16/R17/R18 profiles. No new RED, product/test/fixture behavior,
PostgreSQL/Compose, toolchain/pin, target inventory, coverage threshold, API,
data model, migration, deployment, sprint, task, or FV identity/order/count
change is authorized.

R19 requires fresh completeness, meta, and simplicity plan reviews in exactly
this order, then a pre-token equality gate: the final nested plan review paths
must exist, say PASS, hash-match, and exactly equal the ordered
approval/token review set. The sole post-token review repeats this ordered-set
check and the generation-argument equality. The later token may bind only the
R18 parent token, R18 sole review, confirmed R18 defect, R19 approval,
snapshots, final three plan reviews, and retained Task-3b/Task-4b blocks. It
must not bind R18/R19 manifest, lock, record, source, fixture, its own hash,
post-token review, CCR, runtime, coverage, or FV outputs.

The R19 implementation has exactly three immutable-snapshot plus sixteen
live-worktree guard pairs, with `coverage-r19-exec` the sole coverage
executor. It never opens, hashes, compares, or dispatches R11-R18 runtime
artifacts; the R18 token/review/defect remain token provenance only. The guard
must use explicit required-field/full-exact token schema validation rather
than comparing a metadata-stripped projection to a full token, open and
hash-verify the exact R19 TOCTOU fixture before receipt/dispatch, and require
that fixture to copy itself into each disposable sandbox. Before manifest
lock, the guard must retain fail-closed final-tree/FV checks: exact three
descriptor path maps; FV01-FV08 canonical input order plus
`ignored-r19-recorded-digest`; exact R19 FV07 dispatches; R19 FV09 handoff;
the `task13_r19` final-tree snapshot protocol reference; required G0-F7
plan/requirements/approval/token/review catalog IDs; and absence of old active
IDs.

---

# Task-13 R20 strict successor requirements

## Status and sole purpose

R20 is a candidate-only, append-only authority successor. Its generation is
`task13-r20-g0-f8-token-review-path-consistency-20260907`. It corrects exactly
one immutable R19 defect: R19's plan named a nonexistent `20260906` sole
post-token integrity-review path while the actually issued sole review is dated
`20260907`. R20 must use one exact, consistently dated `20260907` path at every
plan, approval projection, review, guard, fixture, manifest, and runtime
surface. The R20 token is issued before that review and therefore records only
that the future review is unbound; it must not contain the future review path or
hash.

R19 remains byte-preserved historical blocked evidence. No R19 manifest, lock,
coordinator record, receipt, runtime GREEN claim, re-interpretation, or second
R19 post-token review may be created.

## Immutable R19 issuance provenance only

- R19 token: `.agents/results/task-4b-redis-publish-completion-g0-f7-r19-20260906.json`
  (`982a9d0bd606f9b811ebdce3ed09356663ce924eb3c61b2ceaddd6c0fc8aa46b`)
- actual R19 sole review:
  `.agents/results/review-task13-g0-f7-r19-token-integrity-r1-20260907.md`
  (`d3f62441a7e59d9d90a7ed608fe72a4b293ff8ec0ea3fd6c5e3e831fa1cd06ff`)
- confirmed review-path defect:
  `.agents/results/bugs/bug-20260907-r19-token-review-path-mismatch.md`
- confirmed provisional-source defects:
  `.agents/results/bugs/bug-20260907-r19-prelock-source-contract-gaps.md`

The two defect reports have SHA-256 values
`4ed16907f9f2de9aaba624a23572f629d884800ca93f98ab8d746d06875edd63`
and `a907f388718c00ed34ee52983563b278f8d7db5476e34941e558307346e123be`
respectively. Approval and token issuance must bind exactly this ordered pair.

These inputs are issuance provenance only. R20 runtime consumers must not open,
hash, compare, dispatch, or fall back to R11-R19 authority artifacts.

## Inherited fail-closed repairs

The R20 frozen guard/fixture/snapshot implementation must preserve existing
behavior and scope while repairing all confirmed unbound R19 compliance gaps:

1. Validate the complete exact top-level token key set and all critical nested
   token key sets, rather than accepting a subset.
2. Permit snapshot-filter traversal through `.agents` and `.agents/results`,
   while admitting only the exact R20 token, sole review, and coordinator-record
   leaves from that tree.
3. Reject the explicit historical active authority IDs for every G0-F1 through
   G0-F7 generation in the R20 `final_tree_snapshot` catalog, while retaining
   separately classified G0-F1 Task-4b prerequisite evidence. Require the
   exact unique current G0-F8 plan/requirements/approval/token/review ID set.

These repairs are fail-closed authority compliance only. They do not change any
product, infrastructure, test inventory, coverage target/profile, toolchain or
Nix pin, threshold, API, schema, migration, deployment, or command argv.
Every R20 source-audit and status surface must therefore say `R11-R19`, not
the inherited `R11-R18`, and must classify only the R19 token, actual
`20260907` sole review, and ordered two R19 defect reports as issuance
provenance. None of those R19 artifacts is a runtime input.

## Exact R20 completion-token schema

The canonical `token_schema` object is materialized byte-identically at
`assertion_manifests.task13_r20.token_schema` and
`assertion_manifests.task13_r20.manifest_projection.token_schema`. Serialized
alone as compact `jq -cS` JSON plus one LF, its SHA-256 is
`04edfadcbfbecd49f736dff925d88dcb2196d61385a2404943f0f5a962c1b4cc`.
Its embedded placeholder-bearing canonical token template has SHA-256
`efa94d1f689fddc9ac5fbce9343388d4b259d70822e08a0c6e9e625782a9231c`.

The exact top-level key set is `approval_evidence`, `authority_resolution`,
`authority_snapshots`, `completed_at_utc`, `downstream_exclusions`,
`generation_argument_consistency`, `generation_id`, `immutability_and_scope`,
`issuance_freshness`, `owner`, `parent_token`, `prerequisite_id`,
`retained_task3b_checker_correction`, `retained_task4b_prerequisite`,
`schema_version`, `status`, and `supersedes`.

The schema enumerates exact sorted key arrays for all 28 normalized nested
object locations: `/approval_evidence`, its review items and snapshot hash;
`/authority_snapshots` and both approved snapshots; `/downstream_exclusions`;
`/generation_argument_consistency`; `/immutability_and_scope`;
`/issuance_freshness`; `/parent_token`, its integrity review and two defect
items; every object in the retained Task-3b checker block; every object in the
retained Task-4b prerequisite block; and `/supersedes`. Array object indices
normalize to `*`. No unlisted object shape is accepted.

The template fixes every non-dynamic scalar value and JSON type, including
all booleans, integers, Unicode strings, R20/R19 paths, generation values,
statuses, retained Task-3b/Task-4b content, coverage-stream digest, and the
ordered two-defect parent provenance. Exactly ten declared placeholders cover
only issuance time, final plan/requirements hashes, approval hash, and the
three final review paths/hashes. Each placeholder declares its exact JSON
pointer cardinality, source, type, and where applicable lowercase-64-hex or
UTC pattern. Issuance substitutes them only from the current R20 reviewed
bytes; no placeholder or null may remain.

Explicit array rules fix the three ordered PASS plan reviews, empty coverage
change list, five Task-13 IDs, four unchanged coverage command indices, two
ordered R19 defect reports, three retained Task-3b reviews, and eight-name
Task-4b test oracle. Every other array must equal the instantiated template in
length, order, element type, and value.

The frozen guard first verifies the exact R20 token path, byte SHA, regular
non-symlink identity, and canonical compact JSON plus LF. It then compares the
root and every normalized nested object key set to the schema, resolves the
ten placeholders from their declared current R20 sources, requires recursive
exact equality to the instantiated template, checks every array rule, and
requires the two schema copies to be equal. Missing, extra, duplicate,
malformed, reordered, coerced, unsubstituted, or wrong-valued content blocks
before receipt, snapshot, pair selection, or nested dispatch with zero action.
Runtime must never derive this schema from or open any R11-R19 authority file.

## Self-contained inherited authority

The exact R19 requirements bytes precede this R20 amendment. The R20 plan must
likewise retain the complete prior master-plan context and include a fully
materialized `assertion_manifests.task13_r20` projection. That projection copies
every unchanged behavioral contract into R20 authority while rebinding only
R20 generation, digest, guard, source, receipt, descriptor, handoff, catalog,
and operator paths. Runtime may consume only the resulting R20 manifest and
R20 artifacts; it may not reopen R11-R19 authority files.

The R20 projection must include, without relying on historical runtime input:

- coverage expected disposition, clean/retention/profile-growth rules, pinned
  tool provenance, exact target inventories, and the four locked commands;
- deployment-boundary, timing, module-count, module-package, MinIO interface,
  and supported-system behavior unchanged from the inherited contract;
- receipt, shape, guarded-flake-local, coverage, module-eval, builder,
  cross-system, and final-verify operator cards under `<r20-recorded-digest>`;
- exact aarch64-darwin, aarch64-linux, and x86_64-linux terminal, first-receipt,
  and FV07 replay paths dated `20260907`;
- FV01 through FV09 identity and order, exact three FV07 zero-action replay
  calls, the R20 handoff path, complete source-snapshot and ignored-input
  inventory, exact four-entry GPE map and generated-evidence bindings, and the
  canonical FV01-FV08 input-set lineage including
  `ignored-r20-recorded-digest`;
- the exact current G0-F8 plan, requirements, approval, token, and sole-review
  catalog IDs, with every old active G0-F1 through G0-F7 ID rejected;
- FV03's fifth public command is exactly
  `scripts/tasks/task-13/mod.just coverage-r20 <r20-recorded-digest>`, never the
  superseded generic `coverage` recipe.

## Frozen coverage contract

The following four strings are byte-identical to R17, R18, and R19:

1. `CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov clean --workspace`
2. `CARGO_NET_OFFLINE=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov --locked --offline --workspace --all-targets --no-report -- --test-threads=1`
3. `CARGO_NET_OFFLINE=true JAMYE_ENABLE_DEV_FIXTURES=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov --locked --offline --workspace --all-targets --all-features --no-report -- --test-threads=1`
4. `CARGO_NET_OFFLINE=true CARGO_LLVM_COV_TARGET_DIR=target/task-13-llvm-cov cargo llvm-cov report --locked --offline --json --summary-only --fail-under-lines 80`

Their canonical stream is UTF-8 with each exact ordered command followed by
one LF, including one final LF. Its required SHA-256 is
`bb77b0f773f339bc5f5767e57f96f6ad8352a299a53c1ff67f103595b4faa12f`.
Both the R20 coverage authority and its fully materialized manifest projection
must carry this value. The R20 manifest and guard must recompute it before
coverage dispatch and block with zero action on a missing, malformed, or
mismatched value.

## Mandatory issuance order

The R20 R1 completeness review is immutable non-authoritative failure evidence:
`.agents/results/review-task13-g0-f8-r20-plan-completeness-r1-20260907.md`
with SHA-256
`e6f3ecf08a0bbb3e2bffac36a72ec24284011dfb814399d34c5965fa506b9aff`.
The R20 R2 completeness review is also immutable non-authoritative failure
evidence:
`.agents/results/review-task13-g0-f8-r20-plan-completeness-r2-20260907.md`
with SHA-256
`1df818e78f784bcf8e67701ccbc3b7b9a89106239fca7d345aceaef78ccdcb48`.
The R20 R3 completeness review is also immutable non-authoritative failure
evidence:
`.agents/results/review-task13-g0-f8-r20-plan-completeness-r3-20260907.md`
with SHA-256
`b57c7539a4dd1d0a3f421922c5a0ab233a6684e379418cd2c8a453fab7de13f2`.
The R20 R4 completeness review is also immutable non-authoritative failure
evidence:
`.agents/results/review-task13-g0-f8-r20-plan-completeness-r4-20260907.md`
with SHA-256
`bbdcf588c6cd2f16d10d76b2bf0c39981746114db8998a748b243f61fe07653b`.
The R20 R5 completeness review is also immutable non-authoritative failure
evidence:
`.agents/results/review-task13-g0-f8-r20-plan-completeness-r5-20260907.md`
with SHA-256
`1c687020c1de325ad0811c294eeead9c5fd51fdf0aa6213012630a144fe4c145`.
All five failed reviews are excluded from approval/token authority. The
complete three-review sequence restarts on the final R6 candidate bytes.

1. Author the exact R20 candidate plan and these requirements.
2. Fresh R20 completeness, meta, then simplicity R6 reviews PASS against exact
   final candidate bytes.
3. Pre-token equality gate confirms the exact ordered review paths/hashes/PASS
   statuses in plan, approval projection, and token projection; all R20
   generation surfaces equal the one R20 generation.
4. Issue R20 approval and R20 token.
5. Issue exactly one R20 post-token integrity PASS review at the planned
   `20260907` path; it rechecks review-set and generation equality.
6. Author R20 frozen sources, guard, snapshot helper, and TOCTOU fixture with
   the three inherited fail-closed repairs.
7. Issue only then R20 manifest, lock, and coordinator record; complete fresh
   R20 implementation CCR reviews; user-owned receipt/shape/flake/coverage and
   later cards follow.

No Nix, Cargo, Just, build, test, format, Clippy, coverage, module/package,
Git, SCM, network, service, deployment, or production operation belongs to
candidate planning.
