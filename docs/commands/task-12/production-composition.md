# M11a Task-12 production-composition cards

Current executable status: Sprint-2 UoW GREEN is 21/21, the existing messaging
and realtime regressions are 11/11 and 21/21, Sprint-3 composition GREEN is
20/20, migration-chain GREEN is 2/2, and Sprint-4 contract GREEN is 7/7; all
exited `0`. Completed selectors do not need to be rerun.

## Sprint 2 — shared-handle UoW GREEN

Task-12 owns no new transaction port or SQLx adapter. The only parent-module exception
is `src/application/mod.rs` registering the owned
`src/application/transactions/` module with `pub mod transactions;`.

Run the task-1-style stable Just card from inside `nix develop path:.`:

```bash
just --justfile scripts/tasks/task-12/mod.just uow-red
printf 'task_12_uow_red_exit=%s\n' "$?"
```

A valid user-run RED compiled `tests/production_composition.rs`, discovered
twenty-one cases, and exited `101` with only the eighteen named RED assertions: ten
direct-call UoW assertions, seven real PostgreSQL cumulative-boundary
assertions, and the persisted-source-event identity assertion.
The test recorders own the operation trace and record the identity of the opaque
Task-4a handle received by each existing feature repository. GREEN makes every
required real feature call on that same handle, commits once on success, and
rolls back once after any feature failure. SendMessage derives its notification
command only from the `PersistedMessage` returned by the messaging operation;
it is not a source-text check or a composition-reported trace.

The integration target now contains 50 tests in total. Both Sprint-2 cards
explicitly skip the later `composition::` (20) and `migration_chain::` (2)
namespaces. Their frozen historical evidence selected exactly the original 21
Sprint-2 tests before the seven `contract_generation::` cases existed; a
present-day rerun would include those seven cases as well. Completed historical
selectors do not need to be rerun.

The three direct-call cases freeze:

- SendMessage: Task-4a message/event/outbox, then Task-8 media binding, then
  Task-9 message notification/push, then exactly one commit.
- CreateTopic: Task-7 topic/chatroom/bootstrap/announcement/read rows, then
  Task-9 topic notification/push, then exactly one commit.
- MarkConversationRead: Task-6b monotonic marker, then Task-9 bounded clear,
  then exactly one commit.

The same direct-call target also contains all seven required cumulative failure
boundaries: three for SendMessage, two for CreateTopic, and two for
MarkConversationRead. Each asserts the recorder's independent prefix/handle
identity, `(begin, commit, rollback) == (1, 0, 1)`, then clears the test
failure and requires a clean retry to produce the full trace and exactly one
commit. GREEN must make those clean-retry assertions pass without changing
their selector or expectations.

Seven PostgreSQL cases use test-only decorators over the existing SQLx
repositories. Each delegates the selected real operation on the same opaque
handle, then returns that feature's typed error immediately after the write.
The fixture snapshots UoW-owned `messages`, `conversation_events`,
`outbox_events`, `message_media`, `topics`, `chatrooms`, `chatroom_reads`,
`notifications`, unread notification state, and `push_delivery_intents` before
the armed call. Its SendMessage input has `body=None` and exactly one confirmed
chat-scoped audio upload with the matching `FinalizedObject`/
`BindMessageMediaItem`; the snapshot additionally records that upload's
`status`, `bound_message_id`, and `consumed_at` shape as either confirmed and
unbound or bound and consumed. It requires a baseline-equivalent rollback,
including restoration of the upload's unconsumed state, then disarms and
requires the stable-input retry's exact durable delta: one `message_media` row
and exactly one upload bind, without duplicates. The fixture is coherent:
group -> existing topic -> `type='topic'` chatroom, owner and recipient
memberships, recipient installation, and a preexisting topic notification for
the bounded-clear path. An eighth PostgreSQL case sends a
message and then a stable-`client_msg_id` retry, requiring both returned internal
`PersistedMessage.source_event_id` values to equal the one canonical durable
`conversation_events.id`. Three identifier cases verify that notification and
clear commands are derived from prior message/topic/marker identities rather
than independently supplied commands.

Missing symbols, compile failure, environment/connection failure, source-text
assertion, or any failure outside the eighteen named RED assertions is
invalid RED evidence. The user alone runs the command and returns its raw output
and exit code. That valid RED evidence authorized this GREEN implementation;
user-run GREEN evidence remains required for acceptance.

After valid RED evidence, use the unchanged behavioral selector for GREEN:

```bash
just --justfile scripts/tasks/task-12/mod.just uow-green
printf 'task_12_uow_green_exit=%s\n' "$?"
```

GREEN completes every real feature call while preserving the same test selector
and assertions. It returns the canonical created/existing event UUID directly
from `conversation_events`; no PostgreSQL rollback boundary is deferred.
Reproducible all-target/all-feature 80% coverage remains mandatory after
Task-12 and Task-13, per the user-approved disposition.

## Sprint 2 — shared Task-4a regression closure

The UoW GREEN changes Task-4a's internal persisted-message result and the
idempotent-existing PostgreSQL lookup. Before closing Sprint 2, rerun the
pre-existing messaging target outside `recovery::` so the ordinary HTTP
idempotency and concurrent stable-key paths are executable-verified against the
current adapter:

```bash
just --justfile scripts/tasks/task-12/mod.just messaging-regression
printf 'task_12_messaging_regression_exit=%s\n' "$?"
```

A valid result discovers and passes eleven tests with no failures and exit `0`.
In particular, it must include
`c4_preserves_content_idempotency_and_exact_text` and
`c4_matching_header_concurrent_retries_share_one_canonical_commit`. Compile,
environment, database, or unrelated failures do not close the regression gate.

The same Task-4a retry path is consumed by the pre-existing realtime C1 target.
Close that remaining blast-radius gate with the non-ignored realtime suite:

```bash
just --justfile scripts/tasks/task-12/mod.just realtime-regression
printf 'task_12_realtime_regression_exit=%s\n' "$?"
```

A valid result exits `0` and includes
`dev_c1_flows_from_seed_to_rest_outbox_redis_websocket_and_delta`, whose second
POST reuses the same `client_msg_id` and must return the first canonical
message. The separately guarded `redis_recovery::` lifecycle case remains
excluded because it stops and restarts Redis. Compile, PostgreSQL/Redis
connection, timeout, or unrelated failures do not close this gate.

## Sprint 3 — final production composition RED (closed pending isolated review)

Only `composition-red` is authored in this RED-authoring pass. Do not run it until fresh
isolated QA and architecture review accept this scaffold. It calls the exact API factory
used by `src/bin/api.rs`, with a deterministic validated AuthConfig fixture, and the
exact worker factory used by `src/bin/worker.rs`. It does not inspect source text, a
route registry, or a composition-owned trace.

```bash
just --justfile scripts/tasks/task-12/mod.just composition-red
printf 'task_12_composition_red_exit=%s\n' "$?"
```

A valid RED compiles and discovers exactly twenty `composition::` tests, then exits `101`
only because final API handler composition and the fixed push/cleanup worker runners are
absent. The expected failing names are:

- `composition::api_root_matches_the_complete_frozen_selected_method_path_inventory`
- `composition::worker_root_constructs_the_fixed_realtime_push_and_cleanup_runner_set`
- the seven named `composition::http_uow::` cumulative-boundary cases (three SendMessage,
  two CreateTopic, and two MarkConversationRead)
- `composition::http_uow::http_bodyless_zero_uploads_preserves_the_existing_content_error`
- `composition::http_uow::http_bodyless_multiple_uploads_preserves_the_existing_media_error`
- `composition::http_uow::http_bodyless_nonfinalized_upload_preserves_the_existing_media_error`
- `composition::http_uow::http_bodyless_unauthorized_upload_preserves_the_existing_media_error`
- `composition::http_uow::http_topic_message_reuses_the_exact_canonical_event_for_one_notification_and_push`
- `composition::http_uow::http_create_topic_push_uses_topic_created_not_announcement_message_created`
- `composition::http_uow::http_main_chat_message_succeeds_without_a_topic_notification_or_push`
- `composition::http_uow::http_inconsistent_topic_chat_topology_fails_closed_before_notification_persistence`
- `composition::http_uow::http_prebridge_golden_error_envelopes_remain_exact_for_all_three_bridged_posts`

The remaining two `composition::` tests (validated AuthConfig and the bounded
negative canaries) must pass. Thus a valid current RED has exactly eighteen
named behavioral failures, not a compile/configuration/environment failure.

The API inventory contains every frozen ordinary operation: 42 REST method/path pairs
plus the realtime WebSocket. It checks handler-specific responses (liveness JSON,
readiness JSON, auth validation/authentication envelopes, protected-endpoint envelope,
and required WebSocket upgrade) rather than merely accepting a non-404. Axum construction failure is
the duplicate-route witness. The separate canary test requires both
`/__dev/fixtures/seed` and `/api/v1/plugins/task-12-probe` to remain `404`; these are
known regression canaries, not a universal claim about arbitrary plugin paths.

The validated-auth test proves missing or invalid AuthConfig values fail closed before
the exact API root is reached. The worker test constructs the exact binary factory with
validated AppConfig/PushConfig and fails only while its one literal runner set lacks
Task-9 push and Task-11 account cleanup; no worker registry or plugin mechanism is
introduced. The finite cleanup batch is a GREEN construction assertion.

Each HTTP-UoW case creates a guarded `jamye_task_test_` database, seeds the
same coherent topic/audio/recipient fixture used by the Sprint-2 PostgreSQL
cases (fresh command key, exactly one confirmed audio upload, one recipient
installation, and one unread bounded-clear notification), derives a real
production-codec bearer token, and calls the exact API router. A UUID-private
schema owns a fully-qualified sequence/function and a uniquely quoted trigger.
The trigger function—not a PostgreSQL `WHEN` clause—performs the required
fixture-specific joins before it runs `nextval` and raises `P0001`; this keeps
nonmatching rows harmless while avoiding forbidden subqueries in `WHEN`.
`last_value = 1 AND is_called`
survives rollback and proves the intended prefix was reached exactly once.

The exact `AFTER` DML witnesses are: SendMessage core -> fixture-specific
`outbox_events INSERT`; media -> the one `message_media INSERT`; message
notification -> fixture recipient/installation `push_delivery_intents INSERT`;
CreateTopic core -> the main-room announcement `message.created`
`outbox_events INSERT`; topic notification -> its fixture-specific push insert;
MarkConversationRead marker -> the fresh recipient/topic `chatroom_reads
INSERT`; and clear -> `notifications UPDATE OF read_at` for the exact seeded
unread notification with `OLD.read_at IS NULL` and `NEW.read_at IS NOT NULL`.
The core-topic witness follows the topic, topic-chatroom, topic event/outbox,
author marker, announcement message, and announcement event writes.

The tests require canonical `503 database_unavailable`, structured
fixture-scoped durable-row equality after rollback, trigger removal, then an
identical HTTP retry (including SendMessage's matching idempotency header) with
the normal status and response-derived topic/read IDs, exact FKs,
upload/read-state/cardinality. A trigger
proves completion of the selected final SQL DML—not a Rust method return—which
is the explicit server-side limitation. Current RED may stop earlier at the
absent route, which is the named missing-final-composition failure; a
fixture/configuration/database failure is invalid evidence.

### S3 bridge acceptance matrix

This matrix distinguishes what the same-selector RED fixes directly from the
already-existing owner suites that remain authoritative. The selector has
twenty discovered cases: two pass canaries and eighteen named behavioral
acceptance cases. Its composition-golden case contains nine frozen
representative requests (validation, authentication, and success for each of
the three bridged POSTs); the remaining named cases freeze rollback, media,
topology, retry, and error families.

| Contract family | RED fixture / preserved owner suite | Exact observable |
| --- | --- | --- |
| Existing handler dispatch and typed outcome/error bridge | all sixteen http_uow cases | Exact existing POST path reaches the single Task-4a handle; return status/outcome remains typed rather than a second handler/UoW result. |
| Send/topic/read HTTP success and stable retry | seven cumulative-boundary cases plus topic/main/create-topic cases | Created then Existing where applicable; one durable canonical row/event relation after retry. |
| Send bodyless failures | four http_bodyless cases | zero, multiple, non-finalized, and unauthorized upload errors have exact envelopes and request-before/request-after DurableRows equality. |
| Task-8 authoritative media binding | existing Task-8 media owner HTTP/repository suite, retained by this task; valid one-audio SendMessage cumulative cases | Lookup, authorization, chat scope, confirmed/exact-bound retry, and row lock remain in the existing DB path on the same handle. Object storage is not injectable from the production root; no new adapter/port/registry is introduced. GREEN must retain the owner-suite proof that the transaction performs zero object-storage calls. |
| Topic notification source | http_topic_message_reuses_the_exact_canonical_event_for_one_notification_and_push | Topic chat has exactly one notification at the canonical event cursor and exactly one push with the exact message.created source event. |
| Main chat behavior | http_main_chat_message_succeeds_without_a_topic_notification_or_push | Message/media success with notifications and pushes counted independently at zero. |
| Corrupt persisted topology | http_inconsistent_topic_chat_topology_fails_closed_before_notification_persistence | Typed fail-closed envelope and request-before/request-after durable snapshot equality; no fabricated topic ID. |
| CreateTopic authority | http_create_topic_push_uses_topic_created_not_announcement_message_created plus retained Topic UoW retry relation | topic.created is distinct from the announcement message.created; the one push references only topic.created. |
| Frozen composition wire baselines | `http_prebridge_golden_error_envelopes_remain_exact_for_all_three_bridged_posts`, four bodyless cases, seven database-boundary cases, topic/main/topology cases | The nine golden requests pin raw serialized bytes, exact status, and content type for validation/authentication/success across all three POSTs. The other named acceptance cases pin their own durable/error/retry families; together the selector has exactly eighteen behavioral acceptance cases. Only request/resource UUID and timestamp field values are replaced with text sentinels; field order, nulls, code, Korean message, and all static bytes remain exact. |
| Supporting named outcome coverage | `messaging_http::c4_preserves_content_idempotency_and_exact_text`, `messaging_http::c4_auth_and_membership_fail_without_resource_disclosure`, `messaging_http::a_failure_after_message_insert_rolls_back_the_entire_command`, `topics::http::t1_through_t7_http_use_the_locked_authenticated_mobile_shapes`, and `chatrooms::http::c1_c2_and_c3_http_use_exact_authenticated_mobile_shapes` | Existing owner contracts remain required evidence for Messaging membership and idempotency mismatch/conflict, Topics membership/conflict, and Read membership. They complement but do not postpone the already-frozen composition golden baseline. |
| Standalone compatibility | existing uow and postgres Sprint-2 selector (21 cases) | Existing messaging/topics/chatrooms wrappers retain their begin/commit/rollback behavior; the bridge must not replace or bypass them. |

The raw-byte normalizer parses only to locate dynamic request/resource UUID and
timestamp values, then substitutes their JSON string literals in the original
serialized response. It never compares serde_json::Value, so JSON field order
and serialized byte shape are part of the golden contract.

Compile errors, missing test symbols, invalid test configuration, environment or
database connection failures, timeouts, dependency-resolution failures, source-text
assertions, or any failure outside the eighteen named behavioral tests are invalid RED
evidence. The AuthConfig and negative-canary tests must pass. A valid card records the
exact literal invocation, raw output, discovered count, every named failure, and
`task_12_composition_red_exit=101`. The user alone owns that execution and raw evidence.

After a valid RED and explicit implementation authorization, `composition-green` keeps
the exact same selector. It may pass only after final static API/worker wiring mounts
each ordinary Router/runner once, leaves dev fixtures out of default production, wires
concrete Kakao/Google providers and access-token codec from validated `AuthConfig`, and
removes only the two Task-12 `dead_code` expectations.

```bash
just --justfile scripts/tasks/task-12/mod.just composition-green
printf 'task_12_composition_green_exit=%s\n' "$?"
```

The final user-owned run discovered and passed all 20 `composition::` cases,
filtered 23 non-Sprint-3 cases, and exited `0`. That evidence is frozen at
`.agents/results/evidence-task12-composition-green-20260901.md`.

## Sprint 3 — migration-chain GREEN (open)

`migration-chain-green` is open after the valid composition GREEN. It runs two
guarded disposable-PostgreSQL integration cases in the
same `production_composition` target: a fresh empty database must apply exactly `0001`
through `0008`, and an exact `0004` predecessor must gain
`chatrooms.topic_id REFERENCES topics(id)` only while `0005` creates `topics`.

```bash
just --justfile scripts/tasks/task-12/mod.just migration-chain-green
printf 'task_12_migration_chain_green_exit=%s\n' "$?"
```

Valid GREEN evidence discovers and passes exactly two `migration_chain::` tests with
exit `0`. An unsafe database target, absent test environment, connection/migration
failure, compile failure, or a partial/persistent database path is invalid evidence.
The tests create only the `jamye_task_test_` disposable databases enforced by the shared
test helper; no migration file is changed here. Reproducible all-target/all-feature 80%
coverage remains deferred until after Task-13 under the user-approved disposition.

The final user-owned run passed both cases with 41 non-migration cases filtered
and exit `0`. Evidence is frozen at
`.agents/results/evidence-task12-migration-chain-green-20260901.md`.

## Sprint 4 — contract-generation RED, RC materialization, and GREEN complete

`contract-red` observes only artifacts generated into a guarded disposable
directory. It uses the `contract_generation::` child selector in the existing
`production_composition` target; it neither calls the separate `tests/contract`
target nor treats source-text scanning as evidence.

```bash
just --justfile scripts/tasks/task-12/mod.just contract-red
printf 'task_12_contract_red_exit=%s\n' "$?"
```

Valid RED evidence discovers exactly seven `contract_generation::` cases: the
committed-provenance canary passes, while these six named behavioral cases fail
with exit `101` until the separately approved Sprint-4 implementation exists.

1. `generated_inventory_has_exactly_42_rest_operations_and_two_selected_realtime_events`
2. `generated_selected_surface_mapping_has_one_handler_owner_test_and_fixture_per_operation_and_event`
3. `disposable_generation_rejects_unapproved_input_and_destination_before_writing`
4. `allowed_provenance_variants_are_deterministic_and_emit_release_candidate_manifest`
5. `generated_c2_mobile_handoff_preserves_the_exact_sqlite_auth_and_delta_contract`
6. `generated_bodyless_audio_reachability_traces_retry_delivery_history_and_metadata_only_reissue`

The canary locks exactly three committed JSON inputs: dirty and clean
pre-publication both use explicit `server_commit=dirty` with `server_tag=null`;
the future-transition input supplies the explicit authorized commit and tag.
The failure set freezes the selected 42 exact operation-ID/method/templated-path
triples plus unique `message.created` v1 and `topic.created` v1 schemas whose
required `type` discriminator is exactly the matching singleton event name. Every
generated mapping row must exactly equal the test-owned handler, production
route probe, owner behavior-test, and genuine fixture oracle; repository
references must be relative normal-component, non-symlink files. The narrow
new C2 declarative fixture closes the pre-existing H1/H2/U1/U2/U3 fixture gap
without inventing a feature contribution. The same failures pin disposable
input/destination pairing before output, byte-determinism plus
`release_candidate` provenance/checksum, the declarative C2
SQLite/auth/two-phase-delta handoff, and the bodyless one-finalized-audio
reachability through delivery and metadata-only MD4/MD5 reissue.

The test serializes filesystem cases and creates/removes only unique guarded
paths. Approved outputs live under
`$TMPDIR/jamye-task-12-contract-generation/<fixture>/...`; one uniquely named
direct `$TMPDIR` sibling safely proves outside-root rejection. Cross-paired
fixtures, parent traversal, provenance symlinks, and destination symlinks must
all be rejected before any artifact write. Compile/configuration/path errors,
an unexpected discovered count, a failing provenance canary, a test-target
substitution, or any failure other than the six cases above is invalid RED
evidence.

The user-owned RED run discovered seven cases, passed the provenance canary,
failed exactly the six named behavioral cases, filtered 43 non-Sprint-4 cases,
and exited `101`. The raw evidence is frozen at
`.agents/results/evidence-task12-contract-red-20260901.md`.

The GREEN implementation adds a release-candidate profile while preserving the
generic C0 generator. Before the GREEN selector, the user owns one explicit
tracked-tree write gate. This is deliberately not a third Sprint-4 Just card:
the command accepts only the exact workspace `contracts` destination and the
committed dirty provenance fixture, requires the exact 18 owner-contribution
files byte-for-byte before writing, and emits exactly 21 generated RC artifacts
without including those read-only contributions in the manifest or checksum.

```bash
CARGO_NET_OFFLINE=true cargo run --locked --bin generate_contracts -- \
  generate-release-candidate \
  --output contracts \
  --provenance tests/production_composition/fixtures/contract_generation/dirty.json
printf 'task_12_contract_materialize_exit=%s\n' "$?"
```

Valid materialization reports `generated 21 release-candidate contract
artifacts` and `task_12_contract_materialize_exit=0`. A different destination,
provenance input, missing or changed owner contribution, compile failure, or
nonzero exit is invalid and does not open GREEN.

The user-owned materialization produced exactly that 21-artifact message and
exited `0`. Static inspection found a 21-entry RC manifest with 42 unique REST
operation IDs and exactly `message.created`/`topic.created`, while all 18 owner
contribution files remained byte-unmodified. The raw execution evidence is
frozen at `.agents/results/evidence-task12-contract-materialize-20260901.md`.

After valid RED and explicit implementation authorization, the reserved GREEN
card preserves the exact same selector. The successful materialization output
above opens it now:

```bash
just --justfile scripts/tasks/task-12/mod.just contract-green
printf 'task_12_contract_green_exit=%s\n' "$?"
```

The final user-owned GREEN run passed all seven cases, filtered 43
non-Sprint-4 cases, emitted no compiler warning, and exited `0`. The exact
invocation and raw output are frozen at
`.agents/results/evidence-task12-contract-green-20260901.md`. Sprint 4 is
complete. The final G5 format, Clippy, architecture, and aggregate evidence is
also recorded below; no user-run Task-12 card is currently pending.

## G5 — final quality gates

The 2026-09-01 user-approved Option A adds four independently addressable,
check-only Task-1 stable targets. The Task-12 cards below are thin dispatches:
they contain no duplicate Cargo command and leave the historical Task-1
`platform-check` unchanged. Run them sequentially and return each complete raw
output and printed exit line. A nonzero result is a failed gate, not a skip.

| Task-12 card | Named Task-1 stable target | Evidence exit label |
| --- | --- | --- |
| `format-check` | `task-1::format-check` | `task_12_g5_format_exit` |
| `clippy` | `task-1::clippy` | `task_12_g5_clippy_exit` |
| `architecture` | `task-1::architecture` | `task_12_g5_architecture_exit` |
| `aggregate` | `task-1::aggregate` | `task_12_g5_aggregate_exit` |

```bash
just --justfile scripts/tasks/task-12/mod.just format-check
printf 'task_12_g5_format_exit=%s\n' "$?"
```

`format-check` runs `cargo fmt --all -- --check`; it never applies formatting
or rewrites source.

The initial check found formatting drift. The user ran `cargo fmt --all` with
`task_12_g5_format_apply_exit=0`, then reran the unchanged Task-12 card. Its
output showed the exact `task-1::format-check` dispatch and
`task_12_g5_format_exit=0`. The raw remediation and passing gate evidence are
frozen at `.agents/results/evidence-task12-g5-format-20260901.md`. The later
Clippy remediation changed Rust files, so this historical pass must be followed
by one final format apply/check before the quality gate closes.

That final post-Clippy-remediation user run completed with
`task_12_g5_format_reapply_exit=0` and the unchanged Task-12 wrapper again
dispatched `task-1::format-check` with `task_12_g5_format_exit=0`. The current
tree's G5 format gate is complete.

```bash
just --justfile scripts/tasks/task-12/mod.just clippy
printf 'task_12_g5_clippy_exit=%s\n' "$?"
```

`clippy` is locked and offline and checks all targets and all features with
every warning denied.

The first user-owned run reached the exact `task-1::clippy` target and failed
with exit `101`: seven diagnostics came from the legacy C0 generator-module
include and seventeen from `production_composition`. The remediation keeps
strict warnings denied while replacing ordinary test-helper lint debt and
narrowly annotating only the release-candidate functions unused by the legacy
duplicate include. Three independent static cross-reviews passed. Attempt
evidence is frozen at
`.agents/results/evidence-task12-g5-clippy-attempt1-20260901.md`; the unchanged
Clippy card must be rerun after the final format check.

The final user-owned rerun reached the same strict Task-1 target, emitted no
diagnostic, and returned `task_12_g5_clippy_exit=0`. Raw passing evidence is
frozen at `.agents/results/evidence-task12-g5-clippy-20260901.md`; the G5
Clippy gate is complete.

```bash
just --justfile scripts/tasks/task-12/mod.just architecture
printf 'task_12_g5_architecture_exit=%s\n' "$?"
```

`architecture` runs the dedicated architecture target with all features.

The user-owned run passed all four architecture cases with no ignored or
filtered case and returned `task_12_g5_architecture_exit=0`. Raw evidence is
frozen at `.agents/results/evidence-task12-g5-architecture-20260901.md`; the
G5 architecture gate is complete.

```bash
just --justfile scripts/tasks/task-12/mod.just aggregate
printf 'task_12_g5_aggregate_exit=%s\n' "$?"
```

`aggregate` loads the established local test environment, runs the locked
all-target test set in default mode, then repeats it with all features and the
explicit non-production dev-fixture guard. It starts no service. Coverage is
not part of G5 and remains deferred until after Task-13 under the existing
user-approved disposition.

The first user-owned aggregate attempt reached the default all-target phase and
passed the library, binary, account-deletion, architecture, auth, and auth-log
targets. It then failed one of thirteen `chatrooms` cases with PostgreSQL
`23503`: the pagination fixture supplied a random `topic_id` without inserting
its required `topics` parent under the canonical 0005 FK. The all-feature phase
did not run. Attempt evidence is frozen at
`.agents/results/evidence-task12-g5-aggregate-attempt1-20260901.md`; this is a
failed gate, not a skip.
