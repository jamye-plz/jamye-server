# Task-13 R24 authority requirements

Status: candidate only; no runtime authority
Generation: `task13-r24-g0-f12-schema-closure-20260907`

## 1. Purpose and predecessor boundary

R24 is the append-only successor to rejected R23. It changes no product, test,
coverage command, coverage threshold, supported system, package, check, module,
descriptor order, FV identity, or deployment responsibility. It closes only the
six blockers frozen in
`.agents/results/bugs/bug-20260907-r23-plan-contract-gaps.md` with SHA-256
`82ae582ca1f01d1d7a24298b24093866b2afa102d86a614685d71d0607d84c0c`.

The rejected R23 plan and requirements remain immutable planning inputs at
SHA-256 values
`55a67ee191c13b7d124ddc3258667ac56d1b4e943e83181021e5811ae716cff6`
and
`94862e43d56c51151a9613e917887376c19031c3c1fe02bb4382a9519a589cc7`.
Its completeness, meta, and simplicity reviews are immutable FAIL records with
SHA-256 values
`703b36bd97dac55167d8ecf88e0043c08ea953211b82867c513b8ae84f6c3f35`,
`4c6dab5b564a7c96a0138c94c728a2503031e17c4bfee82d0c25926eed1fad12`,
and
`573c1f55f428f6a4a39ac23262ae73bc1a6cc345f18fe1f919c41ac6d51fe957`.

No R23 approval, token, source, manifest, lock, record, receipt, terminal, FV
output, GREEN result, or handoff exists or may be invented. Neither R23 nor R21
is a runtime fallback. The eventual R24 manifest materializes every inherited
runtime field, and runtime consumers open no historical plan or review.

## 2. Unchanged scope

The exact supported systems remain, in order, `aarch64-darwin`,
`aarch64-linux`, and `x86_64-linux`. Each has exactly packages `api` and
`worker`. The allowlist remains exactly 24 unique pairs partitioned into three
immutable-snapshot and twenty-one live-worktree pairs. The seven R24-shape pairs
are the shape assertion plus coverage, module, three descriptor, and final
verification recipes. The frozen source set remains eight rename-equivalent
files.

The four coverage commands, including `--test-threads=1` only after the `--`
separator on collection commands two and three, remain byte-identical to R23.
Their LF-delimited stream SHA-256 remains
`bb77b0f773f339bc5f5767e57f96f6ad8352a299a53c1ff67f103595b4faa12f`.
The line threshold remains 80 percent. Module evaluation retains the exact
eighteen-row ABI and the `s2-skeleton`, `s4-green`, and `s5-final` phase counts.
FV01 through FV09 retain their identities, order, input/output ownership, and
post-SHIP external FV09 boundary.

## 3. Canonical machine evidence

Every JSON evidence record named below must be a regular non-symlink file whose
exact bytes equal `jq -cS` compact JSON plus one LF. Before ordinary parse, a
validator uses `jq --stream` path-event cardinality to reject repeated object
member paths, invalid UTF-8, a non-object root, truncated input, a trailing JSON
value, or trailing bytes. Unknown or missing keys, wrong types, reordered
arrays, and noncanonical bytes exit 2 before authority issuance or dispatch.

### 3.1 Plan-review record

Each of the three fixed R24 plan-review paths contains exactly:

```json
{"generation_id":"task13-r24-g0-f12-schema-closure-20260907","kind":"<completeness|meta|simplicity>","plan_sha256":"<plan_sha256>","requirements_sha256":"<requirements_sha256>","schema_version":"1.0","status":"<PASS|FAIL>"}
```

The path-to-kind mapping is fixed by the plan. Only `PASS` is token-eligible.
The two hashes are independently recomputed from the fixed regular non-symlink
candidate paths. Human findings may be reported in the reviewer message or a
separate non-authoritative bug record; prose is never parsed as authority.

### 3.2 Approval record

Only after all three exact review records say `PASS` and the user explicitly
approves those exact plan/requirements hashes may the coordinator create the
fixed approval path with exactly:

```json
{"decision":"STRICT_APPEND_ONLY_OPTION_A","generation_id":"task13-r24-g0-f12-schema-closure-20260907","plan_reviews":[{"kind":"completeness","path":".agents/results/review-task13-g0-f12-r24-plan-completeness-r1-20260907.json","sha256":"<completeness_sha256>","status":"PASS"},{"kind":"meta","path":".agents/results/review-task13-g0-f12-r24-plan-meta-r1-20260907.json","sha256":"<meta_sha256>","status":"PASS"},{"kind":"simplicity","path":".agents/results/review-task13-g0-f12-r24-plan-simplicity-r1-20260907.json","sha256":"<simplicity_sha256>","status":"PASS"}],"plan_sha256":"<plan_sha256>","requirements_sha256":"<requirements_sha256>","schema_version":"1.0","status":"APPROVED"}
```

Each review hash is recomputed, and every review object is validated against
section 3.1 before approval creation. The record is absent before explicit user
approval and is exclusively created once.

### 3.3 Token-integrity and pre-lock records

The sole token-integrity review is exactly:

```json
{"generation_id":"task13-r24-g0-f12-schema-closure-20260907","kind":"token_integrity","plan_sha256":"<plan_sha256>","requirements_sha256":"<requirements_sha256>","schema_version":"1.0","status":"PASS","token_sha256":"<token_sha256>"}
```

The pre-lock audit is exactly one canonical object with root keys
`authority_inputs`, `frozen_sources`, `generation_id`, `schema_version`, and
`status`. `authority_inputs` is the ordered plan, requirements, approval,
completion-token, and token-integrity-review path/SHA list. `frozen_sources` is
the exact ordered eight-item R24 source path/SHA/type list; each type is literal
`regular_non_symlink`. `generation_id` is R24, `schema_version` is `1.0`, and
`status` is `PASS`. Every hash is recomputed immediately before the audit is
exclusively created.

## 4. Exact token schema

The token is a canonical object with exactly eight root keys: `approval`,
`completed_at`, `generation_id`, `plan_reviews`, `planning_snapshots`,
`predecessor_blocker`, `schema_version`, and `source_placeholders`. It is
deep-equal to the R24 plan's complete `token_template` after substituting only
one RFC3339 UTC timestamp ending in `Z` and the six named lowercase-64-hex
hashes for approval, three reviews, plan, and requirements.

The eight source placeholders are in frozen-source order and contain exactly
`path`, `sha256: null`, and `status: PENDING_AT_TOKEN_ISSUANCE`. The token never
hashes itself, never binds source bytes, and never names a future token review,
pre-lock audit, manifest, lock, record, receipt, terminal, FV output, or
handoff. The copied R24 guard is the runtime validator; the independent
token-integrity reviewer applies the same schema immediately after issuance.

## 5. Frozen source and selected Justfile copy ABI

The frozen-source order is:

1. `scripts/tasks/task-13/r24-shape.just`;
2. `scripts/tasks/task-13/guarded-just-r24.sh`;
3. `scripts/tasks/task-13/strict-entrypoint-r24.sh`;
4. `tests/nix/flake-shape-aarch64-linux-r24.sh`;
5. `scripts/tasks/task-13/flake-source-snapshot-r24.sh`;
6. `scripts/tasks/task-13/final-tree-record-r24.sh`;
7. `scripts/tasks/task-13/final-verify-r24.sh`;
8. `tests/nix/coverage-executor-toctou-r24.sh`.

The operator retains the R24 manifest digest and strict-entrypoint digest,
copies the entrypoint to a fresh external regular non-symlink file, sets mode
`0400`, verifies its hash, exports `TASK13_R24_ENTRYPOINT_SHA256`, and executes
only that copy. The copied entrypoint validates and copies the manifest and
guard only. The copied guard alone owns selected-Justfile and helper copying.

For every R24-shape pair, the guard reads the expected shape hash only from
`.command_source_integrity.sources` in the verified manifest copy. It copies
the shape to a fresh external regular non-symlink mode-0400 file and exports
`TASK13_R24_SELECTED_JUSTFILE_COPY`,
`TASK13_R24_SELECTED_JUSTFILE_SHA256`, and
`TASK13_R24_SELECTED_JUSTFILE_IDENTITY`. The generic
`TASK13_BOUND_JUSTFILE`, `TASK13_BOUND_JUSTFILE_SHA256`, and
`TASK13_BOUND_JUSTFILE_IDENTITY` receipts are byte-equal aliases. The shape has
no global `set working-directory`; dispatch argv is exactly:

```text
just --working-directory <verified-physical-repository-root> --justfile <external-selected-copy> <recipe> <validated-arguments>
```

For the immutable R24 shape pair, the guard first creates and verifies the
immutable source snapshot, then copies `r24-shape.just` from that snapshot and
uses the snapshot as `--working-directory`. For live R24 shape pairs it copies
the manifest-hash-equal live shape and uses the verified live repository as the
working directory. Non-R24 immutable pairs execute their verified Justfile
inside the snapshot directly; non-R24 live pairs retain adjacent read-only-copy
behavior so their own working-directory declaration remains valid.

The guard revalidates the selected source and copy
path/hash/mode/device:inode immediately before Just; the selected recipe
revalidates all receipts before action. No runtime component parses mutable
`r24-shape.just`.

The helper-copy matrix is exact. Immutable-snapshot pairs copy only
`flake-source-snapshot-r24.sh`; coverage and all non-R24 live checks copy no
final helper; module and each descriptor copy only
`final-tree-record-r24.sh`; final verification copies
`final-tree-record-r24.sh` and `final-verify-r24.sh`. Every helper copy is a
fresh external regular non-symlink mode-0400 file verified from the manifest.

The frozen TOCTOU fixture operates only on an isolated temporary repository
replica. It has exactly six modes: `control`, `source-precopy-content`,
`source-postcopy-content`, `source-postcopy-replacement`, `copy-content`, and
`copy-replacement`. `control` exits 0 with exactly one nested dispatch. Each
attack exits 2 with zero nested dispatch and zero persistent repository writes.

## 6. Manifest schema and non-circular handoff

The R24 manifest is canonical JSON plus LF with exactly the twenty root keys in
the plan's `manifest_schema.root_keys`. Sixteen roots begin as a deep copy of
`assertion_manifests.task13_r21.manifest_projection` from the immutable R21
plan at SHA-256
`f04c4bb251a37bd05b827b39b7dc79eb15e283662ea7a1fabc131df866536d7f`.
All R21 generation/path literals are deterministically renamed to R24. The
manifest then wholly replaces `strict_entrypoint`, `guarded_dispatch`, and
`token_schema`, applies the named module/final-tree/descriptor replacements,
and adds exactly `authority_binding`, `command_source_integrity`,
`generation_id`, and `guarded_dispatch` as specified by the plan.

`authority_binding` and `command_source_integrity` are instantiated only from
the plan's complete templates and placeholder-source map. No unspecified key
may be generated. The manifest contains no own SHA-256 value or placeholder.
It records only its fixed path plus the fixed external lock and record paths.
The lock and coordinator record each contain exactly the computed lowercase
manifest SHA-256 plus one LF. The operator-supplied digest must equal the live
manifest hash, copied manifest hash, lock line, and record line before any
receipt or pair selection.

## 7. Module evaluator and helper ABI

The exact module argv remains:

```text
nix eval --impure --json --no-write-lock-file --file tests/nix/module-eval.nix --argstr phase <phase> --argstr repoRoot <physical-repository-root>
```

The sole root key is `results`; it has the exact eighteen ordered R23 names,
and each row has only `detail`, `name`, and `status`. Phase counts and exact
SKIP details are unchanged. Only successful `s5-final` calls the copied final
tree helper once as `prepare-snapshot <manifest-digest>`.

`final-tree-record-r24.sh` accepts exactly:

```text
prepare-snapshot <digest>
verify-snapshot <digest> <FV01|FV02|FV03|FV04|FV05|FV06|FV07|FV08>
run-descriptor <digest> <x86_64-linux|aarch64-linux|aarch64-darwin> <gpe-map-digest> <first|fv07-replay>
```

`final-verify-r24.sh` accepts exactly `<digest> <fv01|fv02|fv03|fv04|fv05|fv06|fv07|fv08>`.
The guard accepts the external lowercase selector and GPE-map digest. The copied
final-verify helper alone maps `fv01`→`FV01`, `fv02`→`FV02`, through
`fv08`→`FV08`, then invokes the copied final-tree helper's `verify-snapshot`
with that uppercase token. Any other spelling, case, count, or mapping blocks
with exit 2 before FV action. Neither the guard nor final-tree helper performs
implicit case conversion.

## 8. Descriptor digest, capability, and terminal schema

The descriptor digest remains SHA-256 over the exact nine R23 fields in the
same NUL-delimited order after deterministic R24 rename: generation, manifest
digest, source snapshot, GPE-map digest, target descriptor ID, pair identity,
internal action identity, capability slot, and output-map-schema SHA-256.
Output-map hashes remain the three fixed R23 values.

For aarch64-linux, the exact `nix build --dry-run --no-link
--print-out-paths --no-write-lock-file` capability command runs before
reservation. Its canonical seven-key capability object is hashed into the
capability slot. Nonzero preflight or input drift blocks with no reservation or
target action. Replay uses only the capability object stored in the successful
terminal and never reruns preflight or action.

R24 replaces the inherited exact terminal `record_fields` with the same ordered
R21 list plus `capability_evidence` immediately after `descriptor_digest`.
For aarch64-linux it is exactly the seven-key canonical capability object. For
x86_64-linux and aarch64-darwin it is JSON null. The field is included in the
self-excluding canonical terminal payload hash, post-action descriptor
recalculation, terminal validation, and FV07 replay validation. It is not added
to receipts; receipts bind the descriptor digest and terminal record hash.

The x86_64-linux nested `flake-linux` stdout transcript remains the exact seven
lines frozen in R23 after R24 generation rename. Direct aarch64-linux and
aarch64-darwin action stdout remains exactly two ordered output paths. Every
selected output line matches
`^/nix/store/[0-9abcdfghijklmnpqrsvwxyz]{32}-[-+._?=A-Za-z0-9]+$`, satisfies
`-e`, and fails `-L`. Thus normal directory and regular-file store outputs are
accepted, while every symlink, including a dangling symlink, is rejected.

Reservation paths and NUL-delimited IDs, immutable terminal/first-receipt/sole
FV07-replay state machine, three-target order, post-action equality, FV05/FV06
audit schemas, and final handoff remain exactly the R23 contract after R24
rename except for the explicit terminal field replacement above.

## 9. Issuance and authorization

The required order is: final plan and requirements; three canonical plan-review
records; explicit user approval; canonical approval record; canonical token;
one canonical token-integrity PASS; eight sources; canonical pre-lock audit;
manifest/lock/record; three implementation PASS reviews; then user-owned
receipt, shape, flake, coverage, module, descriptor, FV, and handoff cards.

Before exact reviewed-plan approval, agent work is limited to static reads,
`jq`, `rg`, `sha256sum`, `cmp`, `bash -n`, candidate planning files, bug records,
and review records. Nix, Cargo, Just, build, test, format, Clippy, coverage,
module/package realization, Git/SCM, service/database/Redis/MinIO mutation,
homelab/production access, deployment, commit, push, and Serena memory creation
remain forbidden or user-owned as already established.
