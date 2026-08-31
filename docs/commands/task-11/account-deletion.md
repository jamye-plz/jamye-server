# M10 Task-11 account-deletion command card

## Sprint-1 purpose

Sprint 1 establishes only a compile-valid absent-surface RED baseline. It makes no U3,
D5/D10 transition, cleanup lease, or push-barrier behavioral claim. Those executable
behavior RED tests belong to Sprints 2, 3, and 4 after their minimum compile-valid
scaffolds exist.

## Sprint-3 executable cleanup RED

Sprint 2 is complete. Sprint 3 freezes cleanup behavior with a compile-valid,
feature-local account-object worker, PostgreSQL cleanup repository, and S3 delete
provider. The first scaffold deliberately returns typed `Unavailable` without a SQL
claim or provider request. This keeps the suite behavior-RED without missing symbols
or source-text scans.

**Current status: Task-11 is complete. The four owning TDD sprints, cleanup-owner
supplemental RED/GREEN, final VERIFY, REFINE, SHIP Steps 14-17, and the 2026-08-31
user final approval passed. The final diff passed formatter apply/check, strict
Clippy, and aggregate 25+4.** The
initial seven-case GREEN exposed a real signer-identity gap during
VERIFY. After a valid supplemental RED, the feature-local provider was corrected to use
validated cleanup credentials. The unchanged selector now passes all eight cases and
returns `task_11_cleanup_green_exit=0`; evidence is preserved in
`.agents/results/evidence-m10-s3-cleanup-supplemental-green-20260831.md`. Task-8's media
port and policy remain unchanged.

One supplemental behavior test freezes the required local boundary: exact media
credential reuse is rejected, a distinct cleanup credential remains redacted, and the
actual internal SigV4 request must be signed by the cleanup identity rather than the
media identity. Before the fix, static adjudication expected the original seven cases to
remain GREEN and only this eighth case to fail at
`DELETE was not signed by the cleanup identity`; the recorded RED matched that matrix.
Arbitrary HTTP 404 remains fail-closed as `UnexpectedResponse`; the locked AWS SDK
documents missing-object `DeleteObject` success as 204 and does not model `NoSuchKey`.
Review evidence is preserved in
`.agents/results/result-m10-s3-supplemental-red-adjudication-20260830.md` and
`.agents/results/result-m10-s3-supplemental-red-final-qa-20260830.md`.

The card loads the guarded local test environment for disposable PostgreSQL and a
scripted loopback S3 server. It is locked and offline for Cargo dependency resolution.
Every asynchronous delete assertion is bounded by an outer timeout so a hanging DELETE
cannot hang the suite.

```bash
just --justfile scripts/tasks/task-11/mod.just cleanup-red
printf 'task_11_cleanup_red_exit=%s\n' "$?"
```

The recorded initial RED compiled the target, discovered seven `cleanup::` tests,
connected only to disposable loopback PostgreSQL and scripted loopback S3, and exited
`101` only on observed cleanup behavior. Missing symbols, compile/migration failures,
unsafe database targets, dependency resolution, and source-text assertions are invalid
evidence.

The frozen cases cover strict configuration with zero side effects; concurrent hanging
deletes and timeout retry; `SKIP LOCKED`, lease expiry and same-owner ABA fencing; retry
metadata/deadline terminalization; terminal monotonicity and late-result safety; signed
internal path-style idempotent S3 DELETE with 403/503 classification and no secret; and
real PostgreSQL plus scripted S3 `503 -> retry -> 204 -> succeeded` followed by an empty
third poll.

The supplemental RED used the same selector and discovered exactly eight tests: the
original seven passed and only
`s3_cleanup_rejects_media_credentials_and_signs_with_its_dedicated_identity` failed at
the signer assertion, with exit `101`. The SDK client was not wired to the cleanup
credential until that raw RED output was recorded.

The first supplemental attempt on `2026-08-31` was invalid: the signer case failed as
expected, but a terminal fixture separately violated
`account_object_deletion_intents_terminal_timestamp_check` because two volatile clock
evaluations differed by one microsecond. The test-only fixture now materializes one DB
clock for creation and terminal timestamps. Evidence and RCA are preserved in
`.agents/results/evidence-m10-s3-cleanup-supplemental-red-attempt1-20260831.md` and
`.agents/results/bugs/bug-20260831-cleanup-terminal-fixture-clock-order.md`. Independent
static QA confirmed the corrected fixture should restore the expected 7-pass/1-signer-
failure matrix before the unchanged RED card was rerun. Review:
`.agents/results/result-m10-s3-supplemental-red-fixture-final-qa-20260831.md`.

The rerun is valid supplemental RED evidence: exactly eight tests executed, the seven
prior cases passed, only the dedicated signer case failed at the expected assertion, and
`task_11_cleanup_red_exit=101` was printed. Evidence:
`.agents/results/evidence-m10-s3-cleanup-supplemental-red-20260831.md`. The narrow GREEN
now constructs the feature-local SDK credentials from the validated cleanup identity;
independent architecture and QA reviews passed with zero findings. Aggregate:
`.agents/results/result-m10-s3-signer-green-static-review-20260831.md`. The subsequent
user-owned `cleanup-green` passed 8/8 with exit `0`; fresh ultrawork VERIFY Steps 6-8
also passed with zero findings. Aggregate VERIFY evidence is preserved in
`.agents/results/result-m10-s3-verify-final-20260831.md`.

`cleanup-green` uses the exact same selector and environment. After valid supplemental
RED evidence, the narrow GREEN used the already-validated cleanup access key and secret
when constructing the feature-local SDK client. The recorded result is 8/8 and exit `0`
without changing Task-8 policy/media code or Task-12 composition.

```bash
just --justfile scripts/tasks/task-11/mod.just cleanup-green
printf 'task_11_cleanup_green_exit=%s\n' "$?"
```

The user alone runs these commands inside `nix develop path:.` and returns raw output
with the printed exit-code line. The agent does not run Cargo, Nix, build, compile,
test, fmt, Clippy, coverage, service, container, migration, or network commands.

## Sprint-4 executable HTTP and push-barrier RED

Sprint 4 adds the exact authenticated `DELETE /api/v1/me` Router and compile-valid
behavior tests. The RED handler deliberately stops after shared bearer extraction and
request-shape validation, returning the canonical `503 database_unavailable` envelope
for every otherwise valid request. It does not call `AccountDeletionService` until the
user-owned RED output is recorded.

Run the HTTP contract card first:

```bash
just --justfile scripts/tasks/task-11/mod.just http-contract-red
printf 'task_11_http_contract_red_exit=%s\n' "$?"
```

Valid RED compiled and discovered exactly three `http::` tests. The authentication,
query/body validation, envelope, and deliberate 503 assertions pass. The 204 deletion
and D5 409 cases fail on the placeholder 503. The cases freeze an empty 204; canonical
401 and 503 envelopes with `details:null` and a request ID; 422 for any query or
nonempty body without rejecting an empty body solely because of Content-Type; D5 zero
mutation; and post-204 repeated DELETE plus profile access returning canonical 401,
refresh/private-state/push cleanup, and retained-content anonymization. Compile failures,
missing symbols, source-text scans, database
connection failures, or failures outside this matrix are invalid RED. Expected raw
exit label was `task_11_http_contract_red_exit=101`. The recorded result was exactly
1 pass and the expected placeholder 503 versus 204/409 failures; evidence is preserved in
`.agents/results/evidence-m10-s4-http-contract-red-20260831.md`.

Then run the interleaving card:

```bash
just --justfile scripts/tasks/task-11/mod.just push-barriers-red
printf 'task_11_push_barriers_red_exit=%s\n' "$?"
```

Valid RED compiled and discovered exactly three `push_barriers::` tests. Each test
initiates deletion through the Router and then exercises the real `PushWorker` and
PostgreSQL push adapter. The bounded cases cover deletion before claim, deletion after
claim but before authorization through a test-only gate, and deletion after provider
start through a test-only blocking provider. No case treats the DELETE status itself as
the barrier assertion. With the 503 placeholder, they fail on observed provider,
preview, durable retry/reclaim, or late-completion behavior. A timeout, compile error,
unsafe database target, missing fixture, or unrelated failure is invalid RED. Expected
raw exit label was `task_11_push_barriers_red_exit=101`. All three cases failed only at
the expected missing-deletion provider/late-success behavior; evidence is preserved in
`.agents/results/evidence-m10-s4-push-barriers-red-20260831.md`.

After both valid RED outputs are recorded, the narrow GREEN commands preserve the exact
same selectors and guarded locked/offline environment:

```bash
just --justfile scripts/tasks/task-11/mod.just http-contract-green
printf 'task_11_http_contract_green_exit=%s\n' "$?"

just --justfile scripts/tasks/task-11/mod.just push-barriers-green
printf 'task_11_push_barriers_green_exit=%s\n' "$?"
```

The user-owned unchanged GREEN selectors subsequently passed 3/3 each with exit `0` and
no warnings. Evidence is preserved in
`.agents/results/evidence-m10-s4-http-contract-green-20260831.md` and
`.agents/results/evidence-m10-s4-push-barriers-green-20260831.md`. The final user-owned
quality cards also passed: format exit `0`, strict Clippy exit `0`, account deletion
25/25, architecture 4/4, and aggregate exit `0`.

## Aggregate quality gate

The executable Task-11 cards ran in this order: `format-check`, `clippy`, then
`aggregate`. The aggregate card ran the complete `account_deletion` target before the
architecture target, so `integration-green` remained only a focused diagnostic and was
not repeated as a separate final gate. Recorded aggregate evidence is exactly 25
account-deletion tests, 4 architecture tests, and exit `0`.

When `format-check` reports rustfmt-only drift, apply the formatter through the owning
Task-11 Just target and immediately rerun the check:

```bash
just --justfile scripts/tasks/task-11/mod.just format
printf 'task_11_format_apply_exit=%s\n' "$?"

just --justfile scripts/tasks/task-11/mod.just format-check
printf 'task_11_format_check_exit=%s\n' "$?"
```

`format` runs `cargo fmt --all` and may rewrite Rust source files. It does not compile,
test, migrate, start services, or access the network. `format-check` remains read-only.
The REFINE rerun recorded both formatter commands with exit `0`; strict Clippy and the
25+4 aggregate had already passed on the same refactor.

The `coverage` card remains fail-closed. The current devShell does not declare
`cargo-llvm-cov`, the Rust toolchain does not include compatible coverage tools, and
Task-12 has not yet composed the final API/worker binaries. Independent architecture
review therefore recommends moving the reproducible all-target/all-feature 80% gate to
the Task-12/Task-13 boundary. The user explicitly approved that plan correction on
2026-08-31. Coverage remains mandatory there, but it is no longer a Task-11 completion
card; no ambient coverage tool counts as evidence.

Final VERIFY found one Task-11-local configuration parity gap after the first G5 pass:
the worker constructor accepted whitespace-only or padded `claim_owner` values that the
PostgreSQL adapter and `0008` constraint reject. A focused supplemental RED extended the
existing invalid-configuration case and failed before the production validation change:

```bash
just --justfile scripts/tasks/task-11/mod.just cleanup-red
printf 'task_11_cleanup_owner_red_exit=%s\n' "$?"
```

Valid supplemental RED discovered the same eight cleanup tests, passed the seven
unaffected tests, and fails only
`worker_rejects_non_strict_or_invalid_config_without_claim_or_provider_calls` because
the collected accepted-invalid list contains exactly `whitespace_owner` and
`padded_owner`. The same case also proves `cleanup worker` with an internal space
remains valid. Compile, dependency, database, timeout, or unrelated failures are
invalid evidence.

That RED was recorded with exit `101`. The minimal GREEN aligned the worker constructor
with the existing repository/migration trim rule and removed the RED harness's stale
import without suppression. The unchanged `cleanup-green` selector then passed 8/8
without warnings and `task_11_cleanup_owner_green_exit=0`.

## Local effects and valid evidence

The two commands force Cargo offline with `CARGO_NET_OFFLINE=true`; they may populate
the configured Cargo target directory from already-cached locked dependencies. They must
not modify source, migrations, lockfiles, Git state, services, containers, or network
resources. A missing cached dependency, compile error, missing test target, or unrelated
assertion failure is not valid RED evidence.

## 1. Complete absent-surface RED

```bash
just --justfile scripts/tasks/task-11/mod.just red
printf 'task_11_red_exit=%s\n' "$?"
```

This is valid only when every Task-11 baseline path is absent: `0008`, application,
ports, PostgreSQL adapter, object-storage adapter, and HTTP static-router surface. The
compile-valid target must fail with this category and Cargo-test exit `101`:

```text
RED: the complete Task-11 baseline is absent: 0008 migration plus application, ports, PostgreSQL, object-storage, and HTTP account-deletion surfaces
task_11_red_exit=101
```

If one or more baseline paths already exist, the target emits `INVALID RED` with the
present path list. That is fail-closed evidence, not a valid Sprint-1 RED and not a
prompt to remove files.

## 2. Optional static migration-absent RED

Run this only while `migrations/0008_account_deletion.sql` is absent, after the first
card's raw output is recorded.

```bash
just --justfile scripts/tasks/task-11/mod.just migration-red
printf 'task_11_migration_red_exit=%s\n' "$?"
```

Expected category and exit:

```text
RED: migrations/0008_account_deletion.sql is absent; task-11 must add the exact-0007 forward-only account-deletion migration
task_11_migration_red_exit=101
```

If `0008` exists, the recipe exits `2` and directs the operator to Sprint 2's executable
migration card. This static absence probe is supplementary only; exact-0007 PostgreSQL
upgrade and controlled-failure rollback are Sprint-2 behavior evidence.

## Deferred cards

Do not run or describe the following as Sprint-1 RED evidence:

- `transition-red`: Sprint 2 executable PostgreSQL D5/D10/atomicity evidence
- `cleanup-red`: Sprint 3 executable lease, timeout, reclaim, ABA, retry, and terminal-state evidence
- `http-contract-red` and `push-barriers-red`: Sprint 4 executable U3 and deletion/push interleaving evidence

This was the historical Sprint-1 deferral list. Migration and all four TDD sprint cards
are now GREEN. The final Task-11 `format-check`, `clippy`, and `aggregate` cards passed;
`integration-green` was subsumed by `aggregate`. The `coverage` recipe intentionally
remains fail-closed pending the explicitly approved cross-task tooling and final-binary
gate described above.

## Sprint-1 TDD evidence template

```text
card: red
command: just --justfile scripts/tasks/task-11/mod.just red
expected_category: complete Task-11 baseline absent
exit_code: AWAITING_USER_EXECUTION

card: migration-red
command: just --justfile scripts/tasks/task-11/mod.just migration-red
expected_category: 0008 migration absent
exit_code: AWAITING_USER_EXECUTION

later_behavior_red_and_green: AWAITING_OWNING_SPRINT
```

## Recorded complete absent-surface RED

On `2026-08-30`, the user-run locked/offline target compiled successfully, selected
exactly one test, and failed only with the documented complete-absence message. The
recorded result was `task_11_red_exit=101`; no partial surface, unrelated assertion,
network access, or lockfile change was observed. Full raw output is preserved in
`.agents/results/evidence-m10-s1-red-20260830.md`.

## Recorded supplementary migration-absent RED

On `2026-08-30`, the user-run locked/offline target selected exactly one test and
failed only with the documented missing-`0008` message. The recorded result was
`task_11_migration_red_exit=101`. This is supplementary static absence evidence;
it does not replace Sprint 2's exact-0007 upgrade and controlled-failure rollback
behavior checks. Full raw output is preserved in
`.agents/results/evidence-m10-s1-migration-red-20260830.md`.

## Sprint-2 executable transition RED

Sprint 1 is frozen by the two recorded absence failures above. After `0008` and the
minimum compile-valid application/port/PostgreSQL scaffold exist, the guarded Task-1
local PostgreSQL must be healthy. If it is not running, start the existing disposable
Compose project with `just task-1 infra-up`; this preserves named-volume bytes and does
not authorize `infra-reset`. Then run:

```bash
just --justfile scripts/tasks/task-11/mod.just transition-red
printf 'task_11_transition_red_exit=%s\n' "$?"
```

Valid RED must compile the `account_deletion` integration target, connect only to the
guarded disposable loopback PostgreSQL database, run the `transition::` cases, and fail
on observed D5/D10/orchestration/rollback behavior. Missing symbols, migration failure,
network dependency resolution, an unsafe database target, or a source-text assertion is
not valid evidence. The current deterministic scaffold is expected to return
`DatabaseUnavailable` before all mutations, so the behavior cases must fail with Cargo
test exit `101`.

The frozen cases cover:

- live ownership-transfer preflight with exact zero mutation and one rollback;
- retained message/topic/bound-media anonymization, event-payload scrubbing, private
  state deletion, and one durable intent per unbound object;
- membership-id ordering plus Task-6 removal and Task-9 privacy-fence calls on the same
  caller-owned handle;
- a late injected repository failure that rolls back earlier removals, fences, tombstone,
  and intents;
- the existing soft-deleted-group membership/FK edge, which D5 must not misclassify as a
  live ownership conflict.

The final case froze a pre-GREEN contract blocker. Existing Task-6 membership removal
accepts only live groups, while the original user row cannot be deleted until archived
group ownership and membership references are cleared. After valid RED, the user chose
`A — Task-11 한정 예외`: the account-deletion repository may directly delete only the
target account's memberships joined to soft-deleted groups and reassign only archived
group ownership to the tombstone. Every live membership still goes through Task-6 then
Task-9 on the caller-owned handle.

Do not run `transition-green` until this raw RED output is recorded. Migration GREEN and
transition GREEN remain separate Sprint-2 evidence cards.

### Invalid attempt: PostgreSQL connection refused

The first `2026-08-30` attempt compiled and discovered all five cases, but every case
failed with loopback PostgreSQL `ConnectionRefused` before fixture or service behavior.
Its printed exit `101` is therefore invalid RED. The output is preserved in
`.agents/results/evidence-m10-s2-transition-red-attempt1-20260830.md`; GREEN remains
blocked pending a healthy-infrastructure rerun.

### Recorded executable transition RED

After the guarded infrastructure passed readiness, the second `2026-08-30` attempt
compiled without warnings, migrated and seeded all five disposable PostgreSQL fixtures,
then failed only at the frozen D5/D10/ordering/rollback behavior assertions. It recorded
`task_11_transition_red_exit=101`. Full evidence is preserved in
`.agents/results/evidence-m10-s2-transition-red-20260830.md`.

### Sprint-2 supplemental transition RED

The first GREEN implementation received two independent static review failures before
any GREEN command was authorized. The supplemental cases freeze only those discovered
gaps:

- a pending occurrence whose source event was authored by the deleting account must no
  longer be claimable, while the live recipient's notification, installation, and an
  unrelated occurrence on the same installation remain intact;
- a target-routed `membership.revoked` control outbox row must retain its immutable
  protocol header and decodable payload identity. Account-deletion payload anonymization
  applies to conversation projections, not realtime-control routing envelopes.

Run the same transition selector once more before either remediation is applied:

```bash
just --justfile scripts/tasks/task-11/mod.just transition-red
printf 'task_11_transition_red_exit=%s\n' "$?"
```

Valid supplemental RED compiles and migrates successfully, discovers seven
`transition::` tests, and fails only on the two review-derived behaviors above with
`task_11_transition_red_exit=101`. A compile failure, PostgreSQL connection failure,
failure of one of the original five frozen transition cases, or any unrelated assertion
is invalid evidence. Do not run `transition-green` until this supplemental raw RED is
recorded and both reviewed defects are remediated.

#### Invalid supplemental attempt: SQLx dynamic SQL compile rejection

The first `2026-08-30` supplemental attempt exited `101` before test discovery. SQLx
0.9 rejected dynamically assembled table-name queries with `SqlSafeStr` errors at the
payload scrub helper. This is invalid RED. The query construction was replaced with
static SQL literals without changing either review-derived behavior, so the unchanged
supplemental RED card must be rerun. Evidence is preserved in
`.agents/results/evidence-m10-s2-transition-supplemental-red-attempt1-20260830.md`.

#### Recorded supplemental transition RED

The rerun compiled and executed all seven cases. The original five transition tests
passed; only the source-authored active occurrence and target-routed control-envelope
cases failed. The result was `task_11_transition_red_exit=101`, so both review-derived
remediations are now authorized. Full evidence is preserved in
`.agents/results/evidence-m10-s2-transition-supplemental-red-20260830.md`.

Both remediations subsequently passed independent static QA/architecture review. The
transition GREEN card passed the same seven cases with exit `0`. A clean-link recheck in
a fresh Rust 1.98.0 devShell also passed 7/7 with exit `0` and emitted no stale
Nix-store linker search-path warning. The environment-only defect and its narrow
package-cache remediation are recorded in
`.agents/results/bugs/bug-20260830-stale-nix-rust-linker-search-path.md`; migration GREEN
is now authorized.

#### Recorded migration GREEN

The user-run `migration-green` card passed all three cases on `2026-08-30`: static
forward-only metadata, exact-0007 disposable upgrade, and controlled-failure full
rollback. It returned `task_11_migration_green_exit=0`; evidence is preserved in
`.agents/results/evidence-m10-s2-migration-green-20260830.md`. Sprint 2 is complete.
Sprint 3 subsequently recorded its initial seven-case RED and GREEN. Its fresh VERIFY
found the cleanup-signer identity defect described above. The supplemental 7-pass/1-fail
RED and corrected 8/8 GREEN are recorded, and the final alignment, security/bug, and
regression reviews all passed. Sprint 3 is complete; Sprint 4 owns the executable U3
HTTP contract and push-barrier interleavings.
