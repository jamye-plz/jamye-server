# Task-13 R23 authority requirements

Status: candidate only; no runtime authority
Generation: `task13-r23-g0-f11-meta-closure-20260907`

## 1. Purpose and predecessor boundary

R23 is an append-only successor to the rejected R22 candidate. It changes no
Task-13 product, Nix module, coverage threshold, coverage command, supported
system, package, check, or final-verification responsibility. It freezes only
the four meta-contract defects found during R22 review.

R22 remains byte-preserved historical planning evidence. The confirmed blocker
is `.agents/results/bugs/bug-20260907-r22-meta-contract-gaps.md`
with SHA-256
`6cc639c5df3639e518f3f08c1284f1a9a6d738ad92048b390682e8e7171104cd`.
No R22 approval, token, source, manifest, lock, coordinator record, receipt,
GREEN result, terminal, FV output, or handoff exists or may be invented. No R22
source is a runtime fallback for R23.

The immutable R22 planning snapshots and reviews are planning inputs only:

- `tests/nix/task-13-r22-authority-plan-20260907.json`
  (`e0cc007b400ab48b491fe1249910d761d7e6f87c8347c44d2bbbf70ea51b5ec7`);
- `tests/nix/task-13-r22-authority-requirements-20260907.md`
  (`927378a81774ed520cd20d2da3cc421638743616c9f60e87d0661a8a797253fe`);
- completeness PASS `53a0acda6803a2a05c20bb2a6e1f6823c5e5fbb944d007a73081d059180d24da`;
- meta FAIL `a32257ac486b2120ab5c8a204735278a25e3c31cfb2789b872b9c3c3dfc8cabc`;
- simplicity PASS `630135501facb1f68d502a23757faf4cc67e969f0dd2d10a8e145ae3bb5cce01`.

The eventual R23 manifest materializes all inherited runtime fields. Runtime
consumers never open R21 or R22 planning/review files.

## 2. Unchanged external surface

The operator still retains two lowercase 64-hex trust anchors: the R23
manifest digest and the R23 strict-entrypoint digest. Every authoritative card
copies `strict-entrypoint-r23.sh` to a fresh external regular file, sets mode
`0400`, compares its SHA-256 to the retained entrypoint digest, exports
`TASK13_R23_ENTRYPOINT_SHA256`, and executes only that copy.

The strict entrypoint accepts only:

```text
<entrypoint-copy> <r23-manifest-digest> receipt
<entrypoint-copy> <r23-manifest-digest> pair <justfile> <recipe> [-- <args>]
```

The exact pair set remains 24 entries: three immutable-snapshot pairs and
twenty-one live-worktree pairs. The six R23 control recipes are
`coverage-r23-exec`, `module-eval-r23-exec`, `linux-builder-r23-exec`,
`aarch64-linux-builder-r23-exec`, `cross-system-verify-r23-exec`, and
`final-verify-r23-exec`, all in manifest-bound `r23-shape.just`. The separate
shape recipe is `aarch64-linux-shape-r23-immutable`.

Argument schemas remain:

- module: exactly one of `s2-skeleton`, `s4-green`, `s5-final`;
- each descriptor: exactly one GPE-map digest, optionally followed by literal
  `fv07-replay`;
- final verify: one exact lowercase FV01-FV08 selector followed by the GPE-map
  digest;
- every other pair: zero recipe arguments.

`scripts/tasks/task-13/mod.just` remains byte-preserved and non-authoritative.

## 3. Frozen source and issuance sequence

The exact ordered R23 frozen-source set is:

1. `scripts/tasks/task-13/r23-shape.just`;
2. `scripts/tasks/task-13/guarded-just-r23.sh`;
3. `scripts/tasks/task-13/strict-entrypoint-r23.sh`;
4. `tests/nix/flake-shape-aarch64-linux-r23.sh`;
5. `scripts/tasks/task-13/flake-source-snapshot-r23.sh`;
6. `scripts/tasks/task-13/final-tree-record-r23.sh`;
7. `scripts/tasks/task-13/final-verify-r23.sh`;
8. `tests/nix/coverage-executor-toctou-r23.sh`.

Issuance order is plan+requirements, three exact-byte plan reviews, explicit
approval, canonical token, one token-integrity review, all eight sources,
independent pre-lock source audit, manifest/lock/record, then implementation
alignment/safety/simplicity PASS reviews. Any failure creates a new append-only
successor. Manifest, lock, and record are never issued speculatively.

## 4. Exact module evaluator ABI

`module-eval-r23-exec` invokes exactly this argv template from the physical live
repository root:

```text
nix eval --impure --json --no-write-lock-file --file tests/nix/module-eval.nix --argstr phase <phase> --argstr repoRoot <physical-repository-root>
```

The wrapper parses stdout once and canonicalizes it as `jq -cS` compact JSON
plus one LF. The canonical object has the sole root key `results`. `results`
has exactly eighteen rows in this exact order:

1. `module-options-present`
2. `api-worker-execstart`
3. `listen-environmentfile-wiring`
4. `migrations-manual-default`
5. `migrations-api-prestart-only`
6. `object-storage-default-disabled`
7. `local-minio-enabled`
8. `local-minio-root-credentials-required`
9. `local-minio-console-loopback`
10. `minio-after-wants-not-requires`
11. `timing-invariant-module`
12. `d12-auth-boundary`
13. `no-bucket-provisioning-unit`
14. `module-homelab-resource-boundary`
15. `secret-path-only`
16. `env-timing-boundary`
17. `deployment-boundary`
18. `final-verify-inventory-boundary`

Every row has exactly the lexicographically sorted keys `detail`, `name`, and
`status`. `status` is one of `PASS`, `FAIL`, `SKIP`. `detail` is empty for
PASS/FAIL. SKIP details are exactly:

- `skipped: Task-13 .env.example triplets not yet authored`;
- `skipped: docs/deployment-nix.md not yet authored`;
- `skipped: final-verify card/index/recipe not yet authored`.

The three frozen projections remain exactly 12 FAIL/3 PASS/3 SKIP,
15 PASS/0 FAIL/3 SKIP, and 18 PASS/0 FAIL/0 SKIP. Any schema, order, name,
status, detail, count, or canonicalization mismatch exits 2 as invalid evidence.
The accepted skeleton phase exits 1 with `BASELINE_RED`; the two green phases
exit 0. Only accepted `s5-final` invokes exactly once:

```text
final-tree-record-r23.sh prepare-snapshot <r23-manifest-digest>
```

and passes through the helper's canonical eight-field result.

## 5. Exact helper and receipt ABI

The copied guard supplies only verified read-only copies and their
manifest-bound hashes through these exact environment names:

- `TASK13_R23_FINAL_TREE_HELPER_COPY`
- `TASK13_R23_FINAL_TREE_HELPER_SHA256`
- `TASK13_R23_FINAL_VERIFY_HELPER_COPY`
- `TASK13_R23_FINAL_VERIFY_HELPER_SHA256`
- `TASK13_R23_STRICT_ENTRYPOINT_COPY`
- `TASK13_R23_STRICT_ENTRYPOINT_SHA256`
- `TASK13_R23_SELECTED_JUSTFILE_COPY`
- `TASK13_R23_SELECTED_JUSTFILE_SHA256`
- `TASK13_R23_SELECTED_JUSTFILE_IDENTITY`
- `TASK13_R23_GPE_MAP_SHA256`

The existing recipe receipts are retained with their generic names:
`TASK13_ASSERTION_GENERATION`, `TASK13_ASSERTION_DIGEST`,
`TASK13_FLAKE_SOURCE_SNAPSHOT`, `TASK13_BOUND_JUSTFILE`,
`TASK13_BOUND_JUSTFILE_IDENTITY`, and `TASK13_BOUND_JUSTFILE_SHA256`.

For every R23-shape pair, the guard obtains the expected `r23-shape.just` hash
only from the verified manifest's `command_source_integrity`, copies that file
to a fresh external temporary regular file, sets mode `0400`, records its
physical device:inode identity, and exports the three exact
`TASK13_R23_SELECTED_JUSTFILE_*` receipts. The generic
`TASK13_BOUND_JUSTFILE*` receipts must be byte-equal aliases of those values.
The R23 shape file contains no global `set working-directory`; the guard invokes
exactly `just --working-directory <verified-execution-root> --justfile
<selected-copy> <recipe> [args]`. It revalidates the live source, selected copy,
mode, identity, and manifest hash immediately before `just` and the recipe
revalidates its copy before action. Thus Just never parses the mutable live R23
shape path. A persistent preselection mutation, post-copy content mutation, or
copy/source path replacement exits 2 with zero nested action.

Inherited non-R23 Justfiles retain the R21 adjacent-copy behavior because their
own global working-directory declarations are part of their frozen behavior;
they are not accepted as substitutes for any R23 control recipe.

All copied sources are absolute regular non-symlink paths, mode `0400`, and
revalidated by SHA-256 and physical identity immediately before use. Nested FV
pairs invoke only the already verified external strict-entrypoint copy with the
same manifest and entrypoint trust anchors. Direct live helper or live guard
execution is never evidence.

`final-tree-record-r23.sh` accepts exactly:

```text
prepare-snapshot <digest>
verify-snapshot <digest> <FV01|FV02|FV03|FV04|FV05|FV06|FV07|FV08>
run-descriptor <digest> <x86_64-linux|aarch64-linux|aarch64-darwin> <gpe-map-digest> <first|fv07-replay>
```

`final-verify-r23.sh` accepts exactly:

```text
<digest> <lowercase-fv01-through-fv08-selector>
```

The positional GPE digest is intentionally absent from the final helper argv;
it is supplied only by verified `TASK13_R23_GPE_MAP_SHA256`.

## 6. Exact descriptor action and output mapping

For x86_64-linux, the sole action is a nested verified strict pair selecting
`scripts/tasks/task-1/mod.just::flake-linux`. Its captured raw stdout must match
the exact transcript schema: the guard's manifest and generation receipt lines,
the literal `This card requires an x86_64-linux builder. Failure to find one is
an M0 blocker, not a skip.`, the literal `built x86_64-linux package outputs:`,
exactly two output-path lines, and the literal `x86_64-linux API and worker
package build passed`, in that order. No other stdout line is accepted. The
first output path maps to `packages.x86_64-linux.api`, the second to
`packages.x86_64-linux.worker`. Any missing, extra, reordered, duplicate,
non-store, or nonexistent output blocks success. A valid store output may be a
directory, regular file, or symlink; its filesystem type is not used as package
identity.

The complete accepted x86_64-linux stdout transcript is therefore exactly:

```text
task13_assertion_manifest_sha256=<active-manifest-digest>
task13_assertion_generation=task13-r23-g0-f11-meta-closure-20260907
This card requires an x86_64-linux builder. Failure to find one is an M0 blocker, not a skip.
built x86_64-linux package outputs:
<packages.x86_64-linux.api-output-path>
<packages.x86_64-linux.worker-output-path>
x86_64-linux API and worker package build passed
```

For aarch64-linux and aarch64-darwin, the exact action argv is respectively:

```text
nix build --no-link --print-out-paths --no-write-lock-file path:.#packages.aarch64-linux.api path:.#packages.aarch64-linux.worker
nix build --no-link --print-out-paths --no-write-lock-file path:.#packages.aarch64-darwin.api path:.#packages.aarch64-darwin.worker
```

Captured stdout must be exactly two distinct existing `/nix/store/...` paths in
the declared installable order; the first maps to `api`, the second to
`worker`. For all three targets, each selected path line must match exactly
`^/nix/store/[0-9abcdfghijklmnpqrsvwxyz]{32}-[-+._?=A-Za-z0-9]+$` and satisfy
`-e` or `-L`. Stderr is retained only as raw hashed evidence. This rule is the sole
attribute-to-output-path interpretation.

## 7. Exact aarch64-linux capability proof

Before reservation or target action, the helper runs exactly:

```text
nix build --dry-run --no-link --print-out-paths --no-write-lock-file path:.#packages.aarch64-linux.api path:.#packages.aarch64-linux.worker
```

Exit 0 is the only capable verdict. Nonzero exit, signal, missing command, or
input drift is a zero-target-action hard blocker, not a SKIP. The helper hashes
raw stdout and stderr and embeds exactly this object in the later terminal
record:

```json
{"command":["nix","build","--dry-run","--no-link","--print-out-paths","--no-write-lock-file","path:.#packages.aarch64-linux.api","path:.#packages.aarch64-linux.worker"],"exit":0,"source_snapshot":"<lowercase-64-hex>","stderr_sha256":"<lowercase-64-hex>","stdout_sha256":"<lowercase-64-hex>","target_system":"aarch64-linux","verdict":"capable"}
```

The object is part of the canonical descriptor digest and terminal payload.
It is not a separate mutable receipt and cannot be substituted by x86_64-linux
or Darwin evidence. A successful dry-run does not claim realization; only the
subsequent exact action and output map do.

### 7.1 Descriptor digest replacement rule

R23 wholly replaces the inherited descriptor-digest rule. For every target,
`descriptor_digest` is SHA-256 of these UTF-8 field values separated by one NUL
and followed by one final NUL, in this exact order:

1. `task13-r23-g0-f11-meta-closure-20260907`;
2. the active manifest digest;
3. the current `source_snapshot`;
4. the verified `gpe_artifact_hash_map_sha256`;
5. the exact target descriptor ID;
6. the literal pair identity (`scripts/tasks/task-13/r23-shape.just::<recipe>`);
7. the exact internal action identity;
8. the capability slot;
9. the output-map-schema SHA-256.

The pair identities are respectively
`scripts/tasks/task-13/r23-shape.just::linux-builder-r23-exec`,
`scripts/tasks/task-13/r23-shape.just::aarch64-linux-builder-r23-exec`, and
`scripts/tasks/task-13/r23-shape.just::cross-system-verify-r23-exec` for
x86_64-linux, aarch64-linux, and aarch64-darwin. The internal action identity is literal
`strict-pair:scripts/tasks/task-1/mod.just::flake-linux` for x86_64-linux and
the space-joined exact action argv from section 6 for the other targets. The
capability slot is literal `none` for x86_64-linux and aarch64-darwin. For
aarch64-linux it is the SHA-256 of the canonical capability object bytes
(`jq -cS` plus one LF). Capability preflight therefore occurs before descriptor
digest calculation; the helper recomputes the repository snapshot after the
preflight and before acquiring the descriptor reservation.

The output-map schema stream is its two attribute names, in order, each
followed by NUL. Its fixed hashes are:

- x86_64-linux: `b1255e5e83efb6751da8d1a0ff50cc5aa19df2faad6f24b50f1b30947958a8c7`;
- aarch64-linux: `e7769ba4d9a8d4c7dc5ef72ae0ddf66695cca2a57fb6a33f31cc71a7d7287de1`;
- aarch64-darwin: `9e47eaae19a001012ae422c76becf1eb9e486df657352a2f668d3be024a5f94f`.

Replay recomputes the same descriptor digest from the capability object stored
in the successful terminal; it never reruns capability preflight or target
action. Any capability-object field change changes its canonical SHA,
descriptor digest, and reservation ID and therefore conflicts rather than
opening a new run.

## 8. Reservation identifiers

The snapshot reservation path is fixed at
`.agents/results/.task-13-r23-snapshot-reservation-20260907`.
Descriptor reservation paths are fixed at:

- `.agents/results/.task-13-r23-x86_64-linux-reservation-20260907`;
- `.agents/results/.task-13-r23-aarch64-linux-reservation-20260907`;
- `.agents/results/.task-13-r23-aarch64-darwin-reservation-20260907`.

The snapshot reservation ID is SHA-256 over canonical NUL-delimited
`generation_id`, `assertion_digest`, and literal `prepare-snapshot`.
Each descriptor reservation ID is SHA-256 over canonical NUL-delimited
`generation_id`, `assertion_digest`, `source_snapshot`,
`gpe_artifact_hash_map_sha256`, `target_descriptor_id`, and
`descriptor_digest`, in that order. `mkdir` is the sole exclusive primitive.
Existing reservation directories always block; there is no TTL, reclaim,
delete, overwrite, or automatic retry.

## 9. FV05 and FV06 canonical audit schemas

The FV05 audit path remains
`.agents/results/task-13-fv05-migration-adr-audit-20260902-090159.json`.
Its canonical object has exactly:

```text
assertion_digest, canonical_chain, generation_id, migration_rows,
production_data_touched, source_snapshot, topic_fk_order_valid, verdict
```

`canonical_chain` is exactly `0001` through `0008`. `migration_rows` is eight
ordered objects with exactly `adr_metadata_present`, `id`, `owner_evidence`,
`path`, `recovery_reference_present`, and `sha256`; booleans must be true,
owner evidence must be a nonempty sorted unique string array, and paths/hashes
must match the repository snapshot. `production_data_touched` is false,
`topic_fk_order_valid` true, and `verdict` `PASS`.

The FV06 audit path remains
`.agents/results/task-13-fv06-c2-provenance-20260902-090159.json`.
Its canonical object has exactly:

```text
ambient_git_used, assertion_digest, checksum_self_excluding, fixture_results,
frozen_source_equal, generation_id, server_commit, server_owned_app_contract_lock,
server_tag, source_snapshot, verdict
```

`server_commit` is `dirty`, `server_tag` is null, the three booleans
`checksum_self_excluding` and `frozen_source_equal` are true while
`ambient_git_used` and `server_owned_app_contract_lock` are false.
`fixture_results` has exactly `clean`, `dirty`, and `future_transition`, each
equal to `PASS`; `verdict` is `PASS`.

Both files are canonical `jq -cS` JSON plus one LF, exclusively created once,
and externally SHA-256 bound as FV outputs. Same-step outputs never become
their own pre-inputs.

## 10. Exact R23 token template and validator

The token is canonical `jq -cS` JSON plus one LF and is deep-equal to this
template after substituting only the angle-bracket values:

```json
{"approval":{"path":".agents/results/task-13-g0-f11-r23-approval-20260907.md","sha256":"<approval_sha256>","status":"APPROVED"},"completed_at":"<rfc3339_utc>","generation_id":"task13-r23-g0-f11-meta-closure-20260907","plan_reviews":[{"kind":"completeness","path":".agents/results/review-task13-g0-f11-r23-plan-completeness-r1-20260907.md","sha256":"<completeness_sha256>","status":"PASS"},{"kind":"meta","path":".agents/results/review-task13-g0-f11-r23-plan-meta-r1-20260907.md","sha256":"<meta_sha256>","status":"PASS"},{"kind":"simplicity","path":".agents/results/review-task13-g0-f11-r23-plan-simplicity-r1-20260907.md","sha256":"<simplicity_sha256>","status":"PASS"}],"planning_snapshots":[{"kind":"plan","path":"tests/nix/task-13-r23-authority-plan-20260907.json","sha256":"<plan_sha256>"},{"kind":"requirements","path":"tests/nix/task-13-r23-authority-requirements-20260907.md","sha256":"<requirements_sha256>"}],"predecessor_blocker":{"path":".agents/results/bugs/bug-20260907-r22-meta-contract-gaps.md","sha256":"6cc639c5df3639e518f3f08c1284f1a9a6d738ad92048b390682e8e7171104cd","status":"CONFIRMED"},"schema_version":"2.0","source_placeholders":[{"path":"scripts/tasks/task-13/r23-shape.just","sha256":null,"status":"PENDING_AT_TOKEN_ISSUANCE"},{"path":"scripts/tasks/task-13/guarded-just-r23.sh","sha256":null,"status":"PENDING_AT_TOKEN_ISSUANCE"},{"path":"scripts/tasks/task-13/strict-entrypoint-r23.sh","sha256":null,"status":"PENDING_AT_TOKEN_ISSUANCE"},{"path":"tests/nix/flake-shape-aarch64-linux-r23.sh","sha256":null,"status":"PENDING_AT_TOKEN_ISSUANCE"},{"path":"scripts/tasks/task-13/flake-source-snapshot-r23.sh","sha256":null,"status":"PENDING_AT_TOKEN_ISSUANCE"},{"path":"scripts/tasks/task-13/final-tree-record-r23.sh","sha256":null,"status":"PENDING_AT_TOKEN_ISSUANCE"},{"path":"scripts/tasks/task-13/final-verify-r23.sh","sha256":null,"status":"PENDING_AT_TOKEN_ISSUANCE"},{"path":"tests/nix/coverage-executor-toctou-r23.sh","sha256":null,"status":"PENDING_AT_TOKEN_ISSUANCE"}]}
```

The eight root keys, every nested object key set, both array orders and
cardinalities, literal values, path strings, statuses, and null placeholders
are exact. The only substitutions are one RFC3339 UTC timestamp ending in `Z`
and the six named lowercase-64-hex hashes. Each hash is recomputed from its
fixed regular non-symlink path; review files must contain an unambiguous PASS
verdict and the same final plan/requirements hashes.

Before ordinary parse, the validator uses `jq --stream` path events to reject
any repeated object-member path, invalid UTF-8, non-object root, truncated
input, or extra JSON value. It then requires byte equality to `jq -cS` plus LF,
validates all placeholder types/cardinalities, instantiates the template from
the six recomputed hashes and timestamp, and requires full deep equality.
The token never hashes itself, never binds a source byte, and never names a
future audit, manifest, lock, record, receipt, terminal, FV output, or handoff.
Only the later manifest replaces the null source placeholders with source
path/type/SHA bindings.

## 11. Inherited protocol

Everything not explicitly replaced above is copied byte-for-byte from
`assertion_manifests.task13_r21.manifest_projection` into the R23 manifest with
only deterministic generation/path renames from `r21`/`R21` to `r23`/`R23`.
The R21 coverage command stream SHA
`bb77b0f773f339bc5f5767e57f96f6ad8352a299a53c1ff67f103595b4faa12f`
is unchanged. Supported systems remain exactly `aarch64-darwin`,
`aarch64-linux`, and `x86_64-linux`; packages remain exactly `api` and
`worker`; checks and module/FV responsibilities remain unchanged.

The inherited four-GPE production order, full repository snapshot, ignored
input oracle, terminal state set, canonical JSON hash protocol, first receipt,
sole FV07 zero-action replay, three-system ordering, FV01-FV08 dispatch order,
final handoff, and external FV09 boundary all remain mandatory.

Where the generated manifest must refer to its own eventual digest,
`authority_binding` and `immutability_handoff` contain the literal
`<r23-recorded-digest>` placeholder. The manifest never contains its own
computed SHA-256. Only the external one-line lock and coordinator record contain
that computed digest, avoiding a circular self-hash.

## 12. Authorization boundary

R23 plan authoring and read-only/static review do not authorize Nix, Cargo,
Just, build, test, format, Clippy, coverage, module/package realization, Git,
SCM, service, database, Redis, MinIO, homelab, production, deployment, commit,
or push actions. Those remain user-owned cards. Source authoring begins only
after all three R23 plan reviews PASS and the user explicitly approves the
exact reviewed R23 plan.
