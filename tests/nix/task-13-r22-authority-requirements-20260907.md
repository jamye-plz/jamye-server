# Task-13 R22 authority requirements

Status: candidate only; no runtime authority
Generation: `task13-r22-g0-f10-runtime-abi-20260907`

## 1. Purpose and predecessor boundary

R22 is an append-only successor to the unissued R21 candidate. It changes no
Task-13 product, Nix module, coverage threshold, coverage command, supported
system, package, check, or final-verification responsibility. It freezes only
the runtime ABIs that R21 left semantically open.

R21 remains byte-preserved historical planning evidence. The confirmed blocker
is `.agents/results/bugs/bug-20260907-r21-runtime-contract-under-specification.md`
with SHA-256
`43db3e9b514d649e006c8bd8bbadfe493bece5806a8bf4d5e9e351f0d7f0c5d9`.
No R21 manifest, lock, coordinator record, receipt, GREEN result, terminal,
receipt, FV output, or handoff exists or may be invented. No R21 source is a
runtime fallback for R22.

The immutable R21 planning snapshots are imported only as planning inputs:

- `tests/nix/task-13-r21-authority-plan-20260907.json`
  (`f04c4bb251a37bd05b827b39b7dc79eb15e283662ea7a1fabc131df866536d7f`);
- `tests/nix/task-13-r21-authority-requirements-20260907.md`
  (`6c5f7b538986f5c47461eb552819e0320822d7444fbb90a7b2f5251edee7340f`).

The eventual R22 manifest materializes all inherited runtime fields. Runtime
consumers never open either R21 planning snapshot.

## 2. Unchanged external surface

The operator still retains two lowercase 64-hex trust anchors: the R22
manifest digest and the R22 strict-entrypoint digest. Every authoritative card
copies `strict-entrypoint-r22.sh` to a fresh external regular file, sets mode
`0400`, compares its SHA-256 to the retained entrypoint digest, exports
`TASK13_R22_ENTRYPOINT_SHA256`, and executes only that copy.

The strict entrypoint accepts only:

```text
<entrypoint-copy> <r22-manifest-digest> receipt
<entrypoint-copy> <r22-manifest-digest> pair <justfile> <recipe> [-- <args>]
```

The exact pair set remains 24 entries: three immutable-snapshot pairs and
twenty-one live-worktree pairs. The six R22 control recipes are
`coverage-r22-exec`, `module-eval-r22-exec`, `linux-builder-r22-exec`,
`aarch64-linux-builder-r22-exec`, `cross-system-verify-r22-exec`, and
`final-verify-r22-exec`, all in manifest-bound `r22-shape.just`. The separate
shape recipe is `aarch64-linux-shape-r22-immutable`.

Argument schemas remain:

- module: exactly one of `s2-skeleton`, `s4-green`, `s5-final`;
- each descriptor: exactly one GPE-map digest, optionally followed by literal
  `fv07-replay`;
- final verify: one exact lowercase FV01-FV08 selector followed by the GPE-map
  digest;
- every other pair: zero recipe arguments.

`scripts/tasks/task-13/mod.just` remains byte-preserved and non-authoritative.

## 3. Frozen source and issuance sequence

The exact ordered R22 frozen-source set is:

1. `scripts/tasks/task-13/r22-shape.just`;
2. `scripts/tasks/task-13/guarded-just-r22.sh`;
3. `scripts/tasks/task-13/strict-entrypoint-r22.sh`;
4. `tests/nix/flake-shape-aarch64-linux-r22.sh`;
5. `scripts/tasks/task-13/flake-source-snapshot-r22.sh`;
6. `scripts/tasks/task-13/final-tree-record-r22.sh`;
7. `scripts/tasks/task-13/final-verify-r22.sh`;
8. `tests/nix/coverage-executor-toctou-r22.sh`.

Issuance order is plan+requirements, three exact-byte plan reviews, explicit
approval, canonical token, one token-integrity review, all eight sources,
independent pre-lock source audit, manifest/lock/record, then implementation
alignment/safety/simplicity PASS reviews. Any failure creates a new append-only
successor. Manifest, lock, and record are never issued speculatively.

## 4. Exact module evaluator ABI

`module-eval-r22-exec` invokes exactly this argv template from the physical live
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
final-tree-record-r22.sh prepare-snapshot <r22-manifest-digest>
```

and passes through the helper's canonical eight-field result.

## 5. Exact helper and receipt ABI

The copied guard supplies only verified read-only copies and their
manifest-bound hashes through these exact environment names:

- `TASK13_R22_FINAL_TREE_HELPER_COPY`
- `TASK13_R22_FINAL_TREE_HELPER_SHA256`
- `TASK13_R22_FINAL_VERIFY_HELPER_COPY`
- `TASK13_R22_FINAL_VERIFY_HELPER_SHA256`
- `TASK13_R22_STRICT_ENTRYPOINT_COPY`
- `TASK13_R22_STRICT_ENTRYPOINT_SHA256`
- `TASK13_R22_GPE_MAP_SHA256`

The existing recipe receipts are retained with their generic names:
`TASK13_ASSERTION_GENERATION`, `TASK13_ASSERTION_DIGEST`,
`TASK13_FLAKE_SOURCE_SNAPSHOT`, `TASK13_BOUND_JUSTFILE`,
`TASK13_BOUND_JUSTFILE_IDENTITY`, and `TASK13_BOUND_JUSTFILE_SHA256`.

All copied sources are absolute regular non-symlink paths, mode `0400`, and
revalidated by SHA-256 and physical identity immediately before use. Nested FV
pairs invoke only the already verified external strict-entrypoint copy with the
same manifest and entrypoint trust anchors. Direct live helper or live guard
execution is never evidence.

`final-tree-record-r22.sh` accepts exactly:

```text
prepare-snapshot <digest>
verify-snapshot <digest> <FV01|FV02|FV03|FV04|FV05|FV06|FV07|FV08>
run-descriptor <digest> <x86_64-linux|aarch64-linux|aarch64-darwin> <gpe-map-digest> <first|fv07-replay>
```

`final-verify-r22.sh` accepts exactly:

```text
<digest> <lowercase-fv01-through-fv08-selector>
```

The positional GPE digest is intentionally absent from the final helper argv;
it is supplied only by verified `TASK13_R22_GPE_MAP_SHA256`.

## 6. Exact descriptor action and output mapping

For x86_64-linux, the sole action is a nested verified strict pair selecting
`scripts/tasks/task-1/mod.just::flake-linux`. Its captured stdout must contain
exactly two distinct regular `/nix/store/...` output-path lines in installable
order. The first maps to `packages.x86_64-linux.api`, the second to
`packages.x86_64-linux.worker`. Any missing, extra, duplicate, non-store, or
non-regular output blocks success.

For aarch64-linux and aarch64-darwin, the exact action argv is respectively:

```text
nix build --no-link --print-out-paths --no-write-lock-file path:.#packages.aarch64-linux.api path:.#packages.aarch64-linux.worker
nix build --no-link --print-out-paths --no-write-lock-file path:.#packages.aarch64-darwin.api path:.#packages.aarch64-darwin.worker
```

Captured stdout must be exactly two distinct regular `/nix/store/...` paths in
the declared installable order; the first maps to `api`, the second to
`worker`. Stderr is retained only as raw hashed evidence. This rule is the sole
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

## 8. Reservation identifiers

The snapshot reservation path is fixed at
`.agents/results/.task-13-r22-snapshot-reservation-20260907`.
Descriptor reservation paths are fixed at:

- `.agents/results/.task-13-r22-x86_64-linux-reservation-20260907`;
- `.agents/results/.task-13-r22-aarch64-linux-reservation-20260907`;
- `.agents/results/.task-13-r22-aarch64-darwin-reservation-20260907`.

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

## 10. Inherited protocol

Everything not explicitly replaced above is copied byte-for-byte from
`assertion_manifests.task13_r21.manifest_projection` into the R22 manifest with
only deterministic generation/path renames from `r21`/`R21` to `r22`/`R22`.
The R21 coverage command stream SHA
`bb77b0f773f339bc5f5767e57f96f6ad8352a299a53c1ff67f103595b4faa12f`
is unchanged. Supported systems remain exactly `aarch64-darwin`,
`aarch64-linux`, and `x86_64-linux`; packages remain exactly `api` and
`worker`; checks and module/FV responsibilities remain unchanged.

The inherited four-GPE production order, full repository snapshot, ignored
input oracle, terminal state set, canonical JSON hash protocol, first receipt,
sole FV07 zero-action replay, three-system ordering, FV01-FV08 dispatch order,
final handoff, and external FV09 boundary all remain mandatory.

## 11. Authorization boundary

R22 plan authoring and read-only/static review do not authorize Nix, Cargo,
Just, build, test, format, Clippy, coverage, module/package realization, Git,
SCM, service, database, Redis, MinIO, homelab, production, deployment, commit,
or push actions. Those remain user-owned cards. Source authoring begins only
after all three R22 plan reviews PASS and the user explicitly approves the
exact reviewed R22 plan.
