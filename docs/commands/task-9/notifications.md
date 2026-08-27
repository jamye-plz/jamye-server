# M8 task-9 notification history and durable Expo push cards

## 목적과 범위

Task-9은 D9=A로 고정된 structured `type+args`와 client localization 계약을 사용해
다음 서버 경계를 구현한다.

- PostgreSQL authoritative notification history와 단건 mark-read
- topic별 bounded coalescing 및 chatroom read cursor 기반 atomic clearing
- Expo-only installation 등록·재바인딩·갱신·삭제
- canonical source event와 installation별 durable push occurrence
- DB-time lease/generation CAS worker 및 privacy-mutation send fence
- N1/N2/P2/P3/P4 DTO·schema·fixture contribution

Task-9은 task-4a의 caller-owned transaction handle, task-6의 live
group/membership locking, task-7의 topic transaction, task-8의 canonical media
projection을 재사용한다. 최종 `SendMessage`, `CreateTopic`,
`MarkConversationRead` production composition은 task-12 소유이며, Task-9은 그
composition이 호출할 정적 notification/push/read operation과 Expo runner만 제공한다.

## 구현 스프린트

1. `0007` migration, canonical field set, D9=A DTO/schema/fixture
2. notification history·pagination·single-read 및 installation owner/epoch HTTP 경계
3. coalescing·topic/message/read atomic operation과 privacy mutation fence
4. claim/reclaim generation CAS, Expo provider 결과, retry/dead-letter, log redaction과 aggregate

각 스프린트는 RED가 실제 assertion 또는 정확한 absent-surface 원인으로 확인된 뒤에만
production 구현과 GREEN으로 진행한다.

## Schema 경계

- External schema: N1/N2/P2/P3/P4와 D9=A `Notification{type,args}` JSON contract
- Conceptual schema: coalesced `notifications`, canonical `push_installations`, canonical
  source-event × installation `push_delivery_intents`
- Internal schema: user/topic/cursor pagination index, global installation/destination unique
  constraints, due/lease/owner-epoch worker index와 terminal-state CHECK

알림·occurrence 생성과 originating topic/message write는 caller-owned transaction 하나에서
ACID로 묶인다. Claim과 provider I/O 사이에는 DB transaction을 유지하지 않으며, claim
generation/live-lease CAS가 stale result를 거절한다.

## 선행 조건

- task-6c와 task-8 aggregate GREEN이 완료돼 있어야 한다.
- D9=A가 machine plan과 `docs/roadmap.md`에 사용자 승인으로 locked돼 있어야 한다.
- `nix develop path:.` devShell 안이어야 한다.
- 최초 RED 실행 시점에는 `migrations/0007_notifications_push.sql`과 Task-9 production
  notification/push surface가 없어야 한다.

## 최초 RED

```bash
just --justfile scripts/tasks/task-9/mod.just red
printf 'task_9_red_exit=%s\n' "$?"
```

유효한 RED는 `tests/notifications.rs`가 현재 locked dependency graph에서 정상
compile된 뒤 다음 원인으로 nonzero 종료한다.

```text
RED: migrations/0007_notifications_push.sql is absent; task-9 must add D9=A structured notification history, Expo-only installation ownership, durable source-event push occurrences, and privacy-fenced delivery
```

예상 종료 코드는 Cargo test failure인 `101`이다. Compile 오류, lockfile 변경, 다른
assertion 실패, 테스트 미발견 또는 migration이 이미 존재해 card가 exit `2`로 거절한
결과는 유효한 RED가 아니다. Raw output을 전달하면 그 증거를 기록한 뒤 `0007` schema와
contract behavior RED로 진행한다.

### 기록된 최초 RED 증거

2026-08-27 사용자 실행에서 `tests/notifications.rs`가 현재 locked graph로 정상
compile된 뒤 위의 정확한 `0007_notifications_push.sql` 부재 문구로 실패했고
`task_9_red_exit=101`이 기록됐다. 따라서 Task-9 구현 게이트는 열렸으며 exact-0006
migration과 D9=A contract behavior RED로 진행한다.

## 부작용과 복구

- Cargo compile/test cache만 만들 수 있다.
- Source, migration, lockfile, PostgreSQL, Redis, external Expo, container와 Git state는
  변경하지 않는다.
- 실제 Expo endpoint나 production credential을 사용하지 않는다.
- 실패 출력은 첫 compile/error/assertion부터 그대로 보존한다.
- RED를 재현하려고 기존 migration, database, volume 또는 source를 삭제하지 않는다.

## Migration RED

```bash
just --justfile scripts/tasks/task-9/mod.just migration-red
printf 'task_9_migration_red_exit=%s\n' "$?"
```

유효한 RED는 test target 전체가 compile된 뒤
`migration::notifications_push_migration_is_forward_only_and_owns_canonical_occurrence_state`
한 테스트가 다음 `0007` 부재 문구로 실패하고 exit `101`을 기록하는 것이다.

```text
RED: migrations/0007_notifications_push.sql is absent; task-9 must add the canonical notification, Expo installation, and per-source-event push occurrence schema
```

이 단계는 PostgreSQL을 사용하지 않는다. 이후 GREEN card만 exact `0006` predecessor
upgrade, 필수 constraint/index, forced-failure transactional rollback을 disposable
PostgreSQL에서 함께 검증한다.

## D9=A contract RED

```bash
just --justfile scripts/tasks/task-9/mod.just contract-red
printf 'task_9_contract_red_exit=%s\n' "$?"
```

유효한 RED는 두 contract test가 발견·compile된 뒤 Task-9 contribution 파일 부재로
실패하고 exit `101`을 기록하는 것이다. 이 RED는 다음을 production 구현보다 먼저
고정한다.

- N1/N2/P2/P3/P4 operation inventory
- public `Notification{type,args,...}`와 server-private field 비노출
- secure-default installation preference와 token/owner epoch 비노출
- identifier-only push-tap handoff와 jamye-app delta-first ownership
- coalesced notification identity와 per-source-event occurrence identity의 분리

Compile 오류, 다른 선행 테스트 실패 또는 stale contribution을 잘못 읽어 통과한 결과는
유효한 RED가 아니다. 두 RED의 raw output을 전달한 뒤 migration과 contract GREEN 구현으로
진행한다.

### 기록된 migration/contract RED 증거

2026-08-27 사용자 실행에서 migration test target이 정상 compile된 뒤 exact
`0007_notifications_push.sql` 부재 문구 하나로 실패했고
`task_9_migration_red_exit=101`이 기록됐다. 같은 실행에서 contract test 두 개가 모두
발견·compile된 뒤 각각 operations와 fixture contribution 부재로 실패했으며
`task_9_contract_red_exit=101`이 기록됐다. Compile 오류나 선행 회귀가 없었으므로 두
behavior RED는 유효하다.

## Migration GREEN

```bash
just --justfile scripts/tasks/task-9/mod.just migration-green
printf 'task_9_migration_green_exit=%s\n' "$?"
```

이 card는 guarded disposable PostgreSQL을 사용해 다음을 검증한다.

- exact `0006` predecessor에서 `0007`로 forward-only upgrade
- `notifications`, `push_installations`, `push_delivery_intents` 세 relation
- global installation/destination uniqueness와 per-source-event occurrence uniqueness
- secure preview default, owner epoch, status/claim/terminal constraints와 access-path indexes
- migration 후 강제 SQL 실패 시 relation과 migration record 전체 rollback

유효한 GREEN은 migration test 3개가 모두 통과하고 exit `0`을 기록한다. 실패한 disposable
database는 test harness가 강제 정리한다. 수동으로 database나 volume을 삭제하지 않는다.

## D9=A contract GREEN

```bash
just --justfile scripts/tasks/task-9/mod.just contract-green
printf 'task_9_contract_green_exit=%s\n' "$?"
```

유효한 GREEN은 contract test 2개가 모두 통과하고 exit `0`을 기록한다. 이 card는
filesystem의 JSON contribution만 읽으며 PostgreSQL, Redis, Expo 또는 container state를
변경하지 않는다.

### 기록된 migration/contract GREEN 증거

2026-08-27 사용자 실행에서 `migration-green`이 exact `0006` predecessor upgrade,
강제 실패 rollback, canonical occurrence schema를 검증하는 migration test 3개를 모두
통과했고 `task_9_migration_green_exit=0`이 기록됐다. 이어서 `contract-green`이 D9=A
operation/schema와 delta-first fixture를 검증하는 contract test 2개를 모두 통과했고
`task_9_contract_green_exit=0`이 기록됐다. Sprint 1은 GREEN이며 Sprint 2의 history와
installation lifecycle behavior RED로 진행한다.

## Notification history orchestration RED

```bash
just --justfile scripts/tasks/task-9/mod.just history-orchestration-red
printf 'task_9_history_orchestration_red_exit=%s\n' "$?"
```

이 card는 외부 서비스나 PostgreSQL 없이 N1/N2 application 경계를 고정한다.

- malformed opaque cursor와 `limit=0|101`은 repository 접근 전에 거절
- authenticated user, opaque after, bounded limit가 정확히 repository query로 전달
- page 크기와 무관한 전체 unread count를 그대로 반환
- N2 첫 호출과 반복 호출이 각각 owner-scoped command 하나와 commit 하나만 사용
- missing/foreign ID가 같은 `notification_not_found` 의미로 rollback

현재 production service는 validation만 제공하고 valid 요청에
`DatabaseUnavailable`을 반환하는 RED scaffold다. 유효한 RED는 target 전체가 compile된 뒤
validation test는 통과하고 나머지 orchestration assertion이 이 scaffold 차이로 실패하며
exit `101`을 기록한다. Compile 오류나 unrelated migration/contract 실패는 유효한 RED가
아니다.

## Expo installation orchestration RED

```bash
just --justfile scripts/tasks/task-9/mod.just installation-orchestration-red
printf 'task_9_installation_orchestration_red_exit=%s\n' "$?"
```

이 card도 외부 Expo와 PostgreSQL을 사용하지 않으며 P2/P3/P4 application 경계를 고정한다.

- P2 omission은 preview `false`, provider는 server-fixed `expo`
- platform/environment/installation/token validation은 side effect 전에 수행
- P3 preview omission은 `None`으로 전달되어 현재 값을 보존
- P3/P4는 authenticated current owner와 전역 installation ID 하나만 전달
- stale owner와 missing installation은 같은 safe not-found 의미로 rollback
- 성공은 begin → one repository command → commit, 실패는 rollback

현재 production service는 validation만 제공하고 valid 요청에
`DatabaseUnavailable`을 반환하는 RED scaffold다. 유효한 RED 조건은 history card와 동일하게
compile 성공 뒤 의도한 behavior assertion만 실패하고 exit `101`을 기록하는 것이다.

### 기록된 Sprint 2 orchestration RED 증거

2026-08-27 사용자 실행에서 두 target 모두 정상 compile됐다. History target은 validation
test 1개가 통과하고 valid N1, N2 success/retry, N2 no-reveal behavior 3개가 정확히
`DatabaseUnavailable` scaffold 차이로 실패해 `task_9_history_orchestration_red_exit=101`을
기록했다. Installation target도 validation test 1개가 통과하고 P2 secure-default, P3/P4
owner-scoped success, stale-owner no-reveal behavior 3개가 같은 scaffold 차이로 실패해
`task_9_installation_orchestration_red_exit=101`을 기록했다. Compile 오류나 unrelated failure가
없으므로 두 RED는 유효하다.

## Sprint 2 application orchestration GREEN

```bash
just --justfile scripts/tasks/task-9/mod.just history-orchestration-green
printf 'task_9_history_orchestration_green_exit=%s\n' "$?"

just --justfile scripts/tasks/task-9/mod.just installation-orchestration-green
printf 'task_9_installation_orchestration_green_exit=%s\n' "$?"
```

두 card는 각각 4개 test가 모두 통과하고 exit `0`이어야 유효한 GREEN이다. 이 단계는
recording repository와 transaction manager만 사용하므로 PostgreSQL이나 Expo에는 접근하지
않는다. History GREEN은 N1 query scope/global unread count와 N2 commit/rollback/no-reveal을,
installation GREEN은 P2 secure default/server-fixed Expo와 P3/P4 current-owner command 및
commit/rollback을 검증한다.

### 기록된 Sprint 2 orchestration GREEN 증거

2026-08-27 사용자 실행에서 history orchestration 4개가 모두 통과해
`task_9_history_orchestration_green_exit=0`이 기록됐고, installation orchestration 4개도
모두 통과해 `task_9_installation_orchestration_green_exit=0`이 기록됐다. Application
validation, query/command 전달, one-transaction commit/rollback, secure default와 no-reveal
error mapping이 GREEN이다.

## PostgreSQL history adapter RED

```bash
just --justfile scripts/tasks/task-9/mod.just history-adapters-red
printf 'task_9_history_adapters_red_exit=%s\n' "$?"
```

Guarded disposable PostgreSQL에 전체 migration을 적용한 뒤 다음을 검증한다.

- N1은 `(created_at,id)` newest-first 순서를 opaque ID cursor로 안정적으로 page 처리
- page와 무관한 authenticated user 전체 unread count
- foreign-user cursor는 `CursorInvalid`
- JSON payload는 D9=A args로 투영
- N2 첫/retry가 같은 `read_at`을 반환하고 owner row 하나만 갱신
- missing/foreign ID는 같은 `NotificationNotFound`
- standalone N2는 sibling notification과 `chatroom_reads`를 변경하지 않음

현재 PostgreSQL notification adapter는 typed `Unavailable` RED scaffold다. 유효한 RED는
두 test가 정상 compile된 뒤 이 scaffold 때문에 실패하고 exit `101`을 기록하는 것이다.

## PostgreSQL installation adapter RED

```bash
just --justfile scripts/tasks/task-9/mod.just installation-adapters-red
printf 'task_9_installation_adapters_red_exit=%s\n' "$?"
```

Guarded disposable PostgreSQL에서 다음 canonical ownership 규칙을 고정한다.

- 동일 binding P2 retry는 같은 row/epoch, owner·environment·token rebind는 epoch 1회 증가
- 새 owner는 기존 preview를 상속하지 않고 command의 explicit 값 또는 application default 사용
- 같은 installation의 environment 이동은 row를 추가하지 않음
- environment+token destination 충돌은 기존 internal row를 보존하며 새 stable identity/owner로 수렴
- P3 preview omission은 보존, token rotation만 epoch 증가
- stale owner/old identity P3/P4는 같은 safe not-found, current owner P4만 삭제

현재 PostgreSQL push adapter는 typed `Unavailable` RED scaffold다. 유효한 RED는 세 test가
정상 compile된 뒤 이 scaffold 때문에 실패하고 exit `101`을 기록하는 것이다. 각 test DB는
harness가 강제 정리하므로 수동 DB/volume 삭제는 하지 않는다.

### 기록된 Sprint 2 PostgreSQL adapter RED 증거

2026-08-27 사용자 실행에서 history adapter test 2개가 모두 정상 compile된 뒤 typed
`Unavailable` scaffold에서만 실패해 `task_9_history_adapters_red_exit=101`을 기록했다.
Installation adapter test 3개도 같은 조건으로 실패해
`task_9_installation_adapters_red_exit=101`을 기록했다. Disposable migration/setup 오류나
다른 assertion 실패가 없었으므로 두 RED는 유효하다.

## Sprint 2 PostgreSQL adapter GREEN

```bash
just --justfile scripts/tasks/task-9/mod.just history-adapters-green
printf 'task_9_history_adapters_green_exit=%s\n' "$?"

just --justfile scripts/tasks/task-9/mod.just installation-adapters-green
printf 'task_9_installation_adapters_green_exit=%s\n' "$?"
```

두 card 모두 guarded disposable PostgreSQL을 사용한다. History GREEN은 test 2개가 모두
통과하고 exit `0`이어야 하며, newest-first cursor page, global unread count, owner-scoped
idempotent read timestamp와 no-reveal을 검증한다. Installation GREEN은 test 3개가 모두
통과하고 exit `0`이어야 하며, identity/destination 수렴, owner epoch fence, secure preview
replacement, P3 omission/token rotation, current-owner P4를 검증한다. 각 disposable database는
test harness가 정리하므로 수동 database 또는 volume 삭제는 하지 않는다.

### 기록된 Sprint 2 PostgreSQL adapter GREEN 증거

2026-08-27 사용자 실행에서 history adapter test 2개가 모두 통과해
`task_9_history_adapters_green_exit=0`이 기록됐다. Installation adapter test 3개도 모두
통과해 `task_9_installation_adapters_green_exit=0`이 기록됐다. Stable notification page와
owner-scoped idempotent read, canonical installation/destination 수렴, owner epoch, preview
보존·교체, current-owner delete가 실제 disposable PostgreSQL에서 GREEN이다. Sprint 2는
완료됐고 Sprint 3의 composable notification/occurrence operation과 privacy fence RED로
진행한다.

## Atomic notification event operation RED

```bash
just --justfile scripts/tasks/task-9/mod.just event-operations-red
printf 'task_9_event_operations_red_exit=%s\n' "$?"
```

이 card는 guarded disposable PostgreSQL에서 Task-12가 caller-owned transaction 안에
조합할 Sprint 3 정적 operation을 먼저 고정한다.

- `topic.created`와 `message.created`는 live group member만 대상으로 fanout
- 같은 topic의 여러 message는 history row 하나로 coalesce하되 source event별 push
  occurrence는 각각 한 개 유지
- 같은 source event 재호출은 notification을 다시 unread로 만들거나 occurrence를
  중복 생성하지 않음
- installation이 없는 member도 history는 받지만 occurrence는 받지 않음
- push preview snapshot은 installation 설정에서 고정하고 message body는 structured
  notification/occurrence payload에 저장하지 않음
- canonical `chatroom_reads.last_read_cursor`까지만 해당 owner/topic notification을 clear
- 다른 topic과 다른 user의 notification은 변경하지 않음

현재 PostgreSQL event repository 세 method는 typed `Unavailable` RED scaffold다. 유효한
RED는 test target 전체가 compile되고 두 test가 모두 이 scaffold의
`DatabaseUnavailable` mapping 때문에 실패하며 exit `101`을 기록하는 것이다. Compile
오류, migration/setup 실패, fixture 오류 또는 다른 assertion 실패는 유효한 RED가 아니다.
각 disposable database는 harness가 정리하므로 수동 database/volume 삭제는 하지 않는다.

### 기록된 Sprint 3 event operation RED 증거

첫 실행은 nested helper 경로의 `E0583` compile 오류였으므로 behavior RED로 인정하지
않았다. 경로를 명시한 뒤 2026-08-27 사용자 재실행에서 target이 정상 compile됐고 두
event-operation test가 모두 typed `DatabaseUnavailable` scaffold에서만 실패해
`task_9_event_operations_red_exit=101`을 기록했다. Migration/setup 또는 unrelated
assertion 실패가 없으므로 이 RED는 유효하다.

## Atomic notification event operation GREEN

```bash
just --justfile scripts/tasks/task-9/mod.just event-operations-green
printf 'task_9_event_operations_green_exit=%s\n' "$?"
```

이 card도 guarded disposable PostgreSQL을 사용한다. 유효한 GREEN은 두 test가 모두
통과하고 exit `0`을 기록하는 것이다. 이를 통해 coalesced notification identity와
per-source-event occurrence identity의 분리, source-event retry 멱등성, installation
preview snapshot, private body 비영속화, canonical cursor 이하 bounded clear를 함께
검증한다. 각 disposable database는 harness가 정리한다.

### 기록된 Sprint 3 event operation GREEN 증거

2026-08-27 사용자 실행에서 event-operation test 2개가 모두 통과했고
`task_9_event_operations_green_exit=0`이 기록됐다. Coalesced history와 source-event별
occurrence 분리, 동일 event retry 멱등성, private body 비영속화, canonical read cursor
이하 bounded clear가 실제 disposable PostgreSQL에서 GREEN이다. 다음 경계는 DB-time
privacy/send-authorization fence RED다.

## DB-time send authorization fence RED

```bash
just --justfile scripts/tasks/task-9/mod.just send-authorization-red
printf 'task_9_send_authorization_red_exit=%s\n' "$?"
```

이 card는 guarded disposable PostgreSQL에서 provider I/O 직전의 최종 authorization과
privacy mutation 선형화 경계를 고정한다.

- live group → membership → recipient notification → installation → occurrence 순서로
  현재 상태를 다시 확인
- notification owner, installation owner/epoch/enabled, claim owner/generation/live lease가
  모두 일치할 때만 identifier-only route와 redacted Expo destination을 반환
- occurrence snapshot과 현재 preference가 모두 true일 때만 canonical message ID를 preview
  파생 단계에 넘기며 raw body는 authorization material에 포함하지 않음
- membership 제거, group 삭제, token/epoch rebind, installation 삭제가 먼저 commit되면
  destination과 message identity를 모두 반환하지 않음
- authorization transaction이 먼저 row lock을 잡으면 rebind가 commit까지 대기하고, 그
  in-flight material 하나만 남으며 이후 같은 claim authorization은 거절
- stale claim owner 또는 generation은 provider material 없이 거절

현재 PostgreSQL authorization method는 typed `Unavailable` RED scaffold다. 유효한 RED는
target 전체가 compile된 뒤 세 test가 이 scaffold의 `Unavailable` 때문에 실패하고 exit
`101`을 기록하는 것이다. Compile 오류, fixture/migration 실패, timeout assertion 실패는
유효한 RED가 아니다. 각 disposable database는 harness가 정리한다.

### 기록된 send-authorization RED 증거

2026-08-27 사용자 실행에서 target 전체가 정상 compile됐고 세 test가 모두 typed
`Unavailable` authorization scaffold에서만 실패했다. 0/3 통과, 21개 filtered out,
`task_9_send_authorization_red_exit=101`이 기록됐으며 timeout이나 fixture/migration 실패는
없었다. 따라서 DB-time authorization과 privacy lock-order GREEN 구현 게이트가 열렸다.

## DB-time send authorization fence GREEN

```bash
just --justfile scripts/tasks/task-9/mod.just send-authorization-green
printf 'task_9_send_authorization_green_exit=%s\n' "$?"
```

유효한 GREEN은 세 test가 모두 통과하고 exit `0`을 기록하는 것이다. 이 단계는 Expo
provider를 호출하지 않고, DB authorization transaction이 내보내는 최소 material과
privacy mutation 전후의 선형화만 검증한다.

### 기록된 send-authorization GREEN 증거

2026-08-27 사용자 실행에서 세 test가 모두 통과했고 21개가 filtered out됐으며
`task_9_send_authorization_green_exit=0`이 기록됐다. 현재 owner/epoch/preview와 live
claim 재검증, mutation-first 전면 거절, authorization-first installation lock 대기 및 이후
attempt 거절이 disposable PostgreSQL에서 GREEN이다.

## Membership/group privacy mutation composition RED

```bash
just --justfile scripts/tasks/task-9/mod.just privacy-mutations-red
printf 'task_9_privacy_mutations_red_exit=%s\n' "$?"
```

이 card는 Task-6c의 실제 `MembershipRevocationService`가 동일 caller-owned transaction에서
group/membership mutation, Task-9 push privacy fence, realtime control intent를 순서대로
조합하는 경계를 검증한다.

- member removal과 group soft-delete가 먼저 commit되면 해당 group의 기존
  pending/claimed/retryable occurrence를 terminal `failed/privacy_revoked`로 전환
- occurrence claim owner와 lease를 제거해 later authorization/retry/result CAS를 차단
- authorization transaction이 먼저 group row를 잡으면 두 production mutation 모두
  commit까지 대기하고, 승인됐던 in-flight material 하나 외에는 이후 material을 반환하지 않음
- notification → installation → occurrence class 순서와 class 내부 UUID 정렬로 잠금
- fence 실패 시 group/membership mutation과 realtime control intent를 함께 rollback

RED 실행 시점의 PostgreSQL privacy fence 두 method는 typed `Unavailable` scaffold였다.
유효한 RED는 target 전체가 compile된 뒤 injected failure rollback test 1개는 통과하고
production fence와 barrier test 2개는 이 scaffold 때문에 실패해 exit `101`을 기록하는
것이다. Compile 오류, setup 실패, send-authorization 회귀 또는 timeout assertion 실패는
유효한 RED가 아니다.

### 기록된 privacy-mutation RED 증거

2026-08-27 사용자 실행에서 target 전체가 compile됐고, injected fence failure rollback test는
통과했다. 두 production test는 모두 typed `Push(Unavailable)` scaffold에서만 실패했으며
24개가 filtered out되고 `task_9_privacy_mutations_red_exit=101`이 기록됐다.

## Membership/group privacy mutation composition GREEN

```bash
just --justfile scripts/tasks/task-9/mod.just privacy-mutations-green
printf 'task_9_privacy_mutations_green_exit=%s\n' "$?"
```

유효한 GREEN은 세 test가 모두 통과하고 exit `0`을 기록한다. 이 단계에서도 provider I/O는
없으며 disposable PostgreSQL transaction과 deterministic barrier만 사용한다.

### 기록된 privacy-mutation GREEN 증거

2026-08-27 사용자 실행에서 세 test가 모두 통과했고 24개가 filtered out됐으며
`task_9_privacy_mutations_green_exit=0`이 기록됐다. fence failure 전체 rollback,
mutation-first occurrence 종결, authorization-first group lock 대기와 이후 retry 차단이
disposable PostgreSQL에서 GREEN이다.

## Push delivery claim/reclaim lifecycle RED

```bash
just --justfile scripts/tasks/task-9/mod.just delivery-lifecycle-red
printf 'task_9_delivery_lifecycle_red_exit=%s\n' "$?"
```

이 card는 provider I/O 없이 PostgreSQL occurrence state machine만 검증한다.

- 동시 claimer는 `FOR UPDATE SKIP LOCKED`로 occurrence 하나를 한 번만 claim
- expired lease reclaim마다 같은 row의 generation을 증가시켜 same-owner ABA 차단
- success/failure completion은 occurrence ID, owner, captured generation, live lease를 모두 비교
- retry는 canonical occurrence를 새로 만들지 않고 같은 row에 attempt/error/next-attempt 기록
- max-attempt 또는 deadline 경계는 durable `dead_letter`와 terminal timestamp 보존
- 비어 있거나 control character를 포함한 owner, zero batch/lease는 row mutation 전 거절

RED 실행 시점의 delivery lifecycle adapter 세 method는 typed `Unavailable` scaffold다.
유효한 RED는 target 전체가 compile된 뒤 다섯 test가 이 scaffold의 `Unavailable`에서만 실패하고
exit `101`을 기록하는 것이다. Fixture/migration 실패, test 미발견, compile 오류 또는 privacy
회귀는 유효한 RED가 아니다.

### 유효하지 않은 첫 delivery-lifecycle RED 시도

2026-08-27 첫 실행은 delivery repository가 `PgPool`을 소유하면서 더 이상 `Copy`가 아닌데도
기존 send-authorization test가 값을 이동한 뒤 다시 사용해 `E0382` compile 오류로 종료됐다.
이는 behavior RED가 아니다. 기존 동시성 test의 독립 task에는 명시적 `clone`을 넘기도록
수정했다.

### 기록된 delivery-lifecycle RED 증거

2026-08-27 사용자 재실행에서 target 전체가 정상 compile됐고 다섯 test가 모두 lifecycle
adapter의 typed `Unavailable` scaffold에서 실패했다. 0/5 통과, 27개 filtered out,
`task_9_delivery_lifecycle_red_exit=101`이 기록됐으며 fixture/migration 또는 privacy 회귀는
없었다. 따라서 이 RED는 유효하다.

## Push delivery claim/reclaim lifecycle GREEN

```bash
just --justfile scripts/tasks/task-9/mod.just delivery-lifecycle-green
printf 'task_9_delivery_lifecycle_green_exit=%s\n' "$?"
```

유효한 GREEN은 다섯 test가 모두 통과하고 exit `0`을 기록한다. 이 단계는 Expo endpoint,
credential, network 또는 새 dependency를 사용하지 않는다.

### 기록된 delivery-lifecycle GREEN 증거

2026-08-27 사용자 실행에서 lifecycle test 다섯 개가 모두 통과했고 27개가 filtered
out됐으며 `task_9_delivery_lifecycle_green_exit=0`이 기록됐다. 동시 claim의
`SKIP LOCKED`, reclaim generation 증가, live-lease completion CAS, same-owner ABA 차단,
retry/dead-letter/deadline 및 invalid claim no-mutation이 disposable PostgreSQL에서 GREEN이다.

## Authorization-first Expo delivery worker RED

```bash
just --justfile scripts/tasks/task-9/mod.just delivery-worker-red
printf 'task_9_delivery_worker_red_exit=%s\n' "$?"
```

이 card는 PostgreSQL, 실제 Expo endpoint, credential 또는 외부 network를 사용하지 않고
recording transaction/repository/preview source/provider만으로 application worker 경계를 고정한다.

- 비어 있거나 control character를 포함한 owner, zero batch, `provider_timeout + margin >= lease`
  구성은 claim 전에 거절
- claim 후 send authorization transaction은 provider와 preview 조회보다 먼저 commit
- authorization 거절은 rollback하고 message-derived text/provider I/O를 모두 생략한 채 terminal
  failure만 generation CAS에 위임
- canonical message body는 authorization commit 뒤에만 읽고 whitespace를 정규화하며 최대
  80 Unicode scalar hint로 제한; request `Debug`는 token과 rendered preview를 redaction
- preview source/Expo outage와 timeout은 success를 쓰지 않고 같은 occurrence에 retry 기록;
  non-retryable provider rejection은 terminal dead-letter
- provider acceptance만 success CAS를 호출
- `DeviceNotRegistered`는 captured claim과 exact destination을 다시 비교하는 transaction에서만
  installation 하나를 disable하며, rebind된 stale feedback은 rollback/no-mutation

현재 `PushWorker::run_once`는 valid 구성에도 typed `RepositoryUnavailable`을 반환하는
compile-valid RED scaffold다. 유효한 RED는 config test 하나는 통과하고 나머지 다섯 behavior
test가 이 scaffold 차이로만 실패해 exit `101`을 기록하는 것이다. Compile 오류, test 미발견,
기존 lifecycle 회귀 또는 timeout assertion 자체의 hang은 유효한 RED가 아니다.

### 기록된 delivery-worker RED 증거

2026-08-27 사용자 실행에서 target 전체가 정상 compile됐고 여섯 test 중 side-effect-free
configuration test 하나가 통과했다. 나머지 다섯 behavior test는 모두
`PushWorker::run_once`의 typed `RepositoryUnavailable` scaffold와 기대 report의 차이에서만
실패했다. 32개가 filtered out됐고 `task_9_delivery_worker_red_exit=101`이 기록됐으며 compile,
fixture, lifecycle 또는 timeout-hang 실패는 없었다. 따라서 GREEN 구현 게이트가 열렸다.

## Authorization-first Expo delivery worker GREEN

```bash
just --justfile scripts/tasks/task-9/mod.just delivery-worker-green
printf 'task_9_delivery_worker_green_exit=%s\n' "$?"
```

유효한 GREEN은 여섯 test가 모두 통과하고 exit `0`을 기록하는 것이다. Recording fake만
사용하므로 실제 Expo endpoint, credential, PostgreSQL 또는 외부 network에는 접근하지 않는다.

### 기록된 delivery-worker GREEN 증거

2026-08-27 사용자 실행에서 worker test 여섯 개가 모두 통과했고 32개가 filtered out됐으며
`task_9_delivery_worker_green_exit=0`이 기록됐다. Authorization commit 이후 preview 파생,
provider timeout/retry와 terminal rejection, acceptance-only success CAS,
`DeviceNotRegistered` transaction 분기가 recording 경계에서 모두 GREEN이다.

## PostgreSQL preview and invalid-destination adapter RED

```bash
just --justfile scripts/tasks/task-9/mod.just delivery-adapters-red
printf 'task_9_delivery_adapters_red_exit=%s\n' "$?"
```

이 card는 guarded disposable PostgreSQL에서 worker가 의존하는 마지막 두 persistence 경계를
고정한다. 실제 Expo endpoint, credential 또는 외부 network는 사용하지 않는다.

- preview source는 authorization이 넘긴 canonical message ID로 nullable `messages.body`만
  조회하고, media-only 또는 missing message는 `None`으로 반환
- `DeviceNotRegistered` feedback은 live claim owner/generation/lease, occurrence의 installation
  owner epoch, 현재 enabled installation과 captured environment/token이 모두 일치할 때만 적용
- 성공 시 exact installation 하나만 disable하고 같은 occurrence를
  terminal `failed/device_not_registered`로 전환하며 claim material을 제거
- 같은 user의 다른 installation과 notification history는 유지
- provider call 뒤 token rebind 또는 claim generation 변경이 먼저 commit됐으면 늦은 feedback은
  `false`를 반환해 caller가 rollback하고 현재 destination을 변경하지 않음

현재 PostgreSQL preview와 invalid-destination method는 typed `Unavailable`을 반환하는
compile-valid RED scaffold다. 유효한 RED는 target 전체가 compile되고 세 test가 모두 이
scaffold 원인으로 실패해 exit `101`을 기록하는 것이다. Compile 오류, migration/setup 실패,
다른 assertion 실패 또는 Expo network 접근은 유효한 RED가 아니다. 각 disposable database는
test harness가 정리하므로 수동 database나 volume을 삭제하지 않는다.

### 기록된 delivery-adapter RED 증거

2026-08-27 사용자 실행에서 target 전체가 정상 compile됐고 세 test가 모두 typed
`Unavailable` scaffold에서만 실패했다. 0/3 통과, 38개가 filtered out됐으며
`task_9_delivery_adapters_red_exit=101`이 기록됐다. Migration/setup, unrelated assertion 또는
provider network 실패가 없었으므로 preview와 invalid-destination GREEN 구현 게이트가 열렸다.

## PostgreSQL preview and invalid-destination adapter GREEN

```bash
just --justfile scripts/tasks/task-9/mod.just delivery-adapters-green
printf 'task_9_delivery_adapters_green_exit=%s\n' "$?"
```

유효한 GREEN은 세 test가 모두 통과하고 exit `0`을 기록하는 것이다. 이 단계에서도 provider
network는 호출하지 않으며, DB-time exact destination/generation CAS와 nullable canonical
preview source만 검증한다.

### 기록된 delivery-adapter GREEN 증거

2026-08-27 사용자 실행에서 PostgreSQL preview와 invalid-destination adapter test 세 개가
모두 통과했고 38개가 filtered out됐으며 `task_9_delivery_adapters_green_exit=0`이 기록됐다.
Canonical nullable message body 조회, exact current destination terminalization, stale rebind 및
claim generation feedback rollback이 모두 GREEN이다.

## N1/N2 and P2-P4 HTTP Router RED

```bash
just --justfile scripts/tasks/task-9/mod.just http-red
printf 'task_9_http_red_exit=%s\n' "$?"
```

이 card는 guarded disposable PostgreSQL에서 static Axum Router가 application/PostgreSQL
경계를 정확히 조립하는지 고정한다. 실제 Expo endpoint, credential 또는 외부 network는
사용하지 않는다.

- 모든 endpoint는 Bearer access identity를 요구
- N1은 `after+limit`의 중복·unknown·malformed·범위 밖 query를 `422`로 거절하고 D9=A
  structured page만 반환
- N2는 owner-scoped idempotent `204`, missing/foreign ID에는 같은
  `404 notification_not_found` envelope를 반환
- P2는 create `201`, canonical upsert `200`, secure preview default `false`를 반환
- P3/P4는 current owner만 update/delete하고 stale owner와 missing identity에 같은 safe `404`
- public JSON은 token, user ID, internal row ID, owner epoch, payload/dedup material을 노출하지 않음

현재 두 Router는 인증, strict query/path/body parsing과 stable error envelope까지만 구현하고
valid request에는 typed `database_unavailable`을 반환하는 compile-valid RED scaffold다. 유효한
RED는 다섯 test가 발견되고 boundary test 하나는 통과하며 N1, N2, P2, P3/P4 behavior 네
test가 모두 scaffold의 `503`과 기대 status 차이에서만 실패해 exit `101`을 기록하는 것이다.
Compile 오류, migration/setup 실패, 인증/validation test 실패 또는 Expo network 접근은
유효한 RED가 아니다. 각 disposable database는 test harness가 정리하므로 수동 database나
volume을 삭제하지 않는다.

### 기록된 HTTP Router RED 증거

2026-08-27 사용자 실행에서 target 전체가 정상 compile됐고 다섯 test가 발견됐다. Bearer,
ambiguous/unknown query, malformed path, strict body를 검증하는 boundary test 하나는 통과했고
N1, N2, P2, P3/P4 behavior 네 test는 각각 scaffold의 `503`과 기대 `200`, `204`, `201`,
`200` status 차이에서만 실패했다. 41개가 filtered out됐고
`task_9_http_red_exit=101`이 기록됐으며 migration/setup, auth/validation 또는 external Expo
실패가 없었으므로 HTTP Router GREEN 구현 게이트가 열렸다.

## N1/N2 and P2-P4 HTTP Router GREEN

```bash
just --justfile scripts/tasks/task-9/mod.just http-green
printf 'task_9_http_green_exit=%s\n' "$?"
```

유효한 GREEN은 HTTP test 다섯 개가 모두 통과하고 exit `0`을 기록하는 것이다. GREEN은
Router가 기존 application service에만 위임하고 D9=A response DTO를 직렬화하며 Task-12의
최종 process composition 소유권은 건드리지 않는다.

### 기록된 HTTP Router GREEN 증거

2026-08-27 사용자 실행에서 HTTP test 다섯 개가 모두 통과했고 41개가 filtered out됐으며
`task_9_http_green_exit=0`이 기록됐다. Bearer/strict input 경계, owner-scoped N1/N2,
current-owner P2-P4, stable status/envelope와 private-field 비노출이 모두 GREEN이다.

## Expo Push Service adapter RED

```bash
just --justfile scripts/tasks/task-9/mod.just expo-adapter-red
printf 'task_9_expo_adapter_red_exit=%s\n' "$?"
```

이 card는 외부 Expo나 production credential을 사용하지 않고 loopback scripted server만으로
실제 provider HTTP 경계를 고정한다.

- 단일 POST는 Expo token과 identifier-only `type/notification_id/conversation_id/message_id`
  data만 전송하고, authorization 뒤에 파생된 선택적 preview만 `body`에 추가
- optional Expo enhanced-security Bearer token을 전송하되 provider Debug와 구조화 로그에서는
  token, Authorization 값, Expo destination, message body와 rendered preview를 모두 제거
- `ok` ticket은 accepted, `DeviceNotRegistered` ticket은 exact installation disable 신호,
  다른 ticket/4xx는 terminal rejection, 429/5xx/malformed provider response는 retryable outage
- redirect를 따르지 않고 HTTPS 또는 명시적 loopback HTTP의 exact send path만 허용

현재 adapter는 입력 URL과 secret redaction만 검증하고 모든 send를 typed `Unavailable`로
반환하는 compile-valid RED scaffold다. 유효한 RED는 네 test가 발견되고 configuration test
하나는 통과하며 request mapping, result classification, structured-log test 세 개가 scaffold의
`Unavailable`과 기대 결과 차이에서만 실패해 exit `101`을 기록하는 것이다. Compile 오류,
외부 network 접근 또는 secret 출력은 유효한 RED가 아니다.

### 기록된 Expo adapter RED 증거

2026-08-27 사용자 실행에서 target이 정상 compile됐고 네 test가 발견됐다. HTTPS/loopback
경계와 access-token Debug redaction test 하나는 통과했고, request mapping 및 structured-log
test는 첫 accepted 결과가 scaffold의 `Unavailable`인 차이에서 실패했다. Classification test도
여섯 결과가 모두 scaffold `Unavailable`인 차이에서만 실패했다. 46개가 filtered out됐고
`task_9_expo_adapter_red_exit=101`이 기록됐으며 외부 network나 secret 출력은 없었다.

## Expo Push Service adapter GREEN

```bash
just --justfile scripts/tasks/task-9/mod.just expo-adapter-green
printf 'task_9_expo_adapter_green_exit=%s\n' "$?"
```

유효한 GREEN은 네 test가 모두 통과하고 exit `0`을 기록하는 것이다. 단일 Expo message는
identifier-only route와 선택적 preview만 전송하고, `ok` ticket ID가 확인된 뒤에만 accepted로
분류한다. `DeviceNotRegistered`, terminal rejection, retryable HTTP/decode failure와 structured
log redaction도 같은 loopback server에서 검증하며 실제 Expo network에는 접근하지 않는다.

### 기록된 Expo adapter GREEN 1차 시도

2026-08-27 사용자 실행에서 request mapping, ticket/HTTP failure classification과 configuration
test 세 개는 통과했다. Structured-log test도 accepted와 retryable 결과까지는 일치했지만
captured JSON에 INFO 성공 이벤트 `expo_push_accepted`가 없어 실패했다. 46개가 filtered out됐고
`task_9_expo_adapter_green_exit=101`이 기록됐으므로 아직 GREEN이 아니다. Process filter와
무관하게 같은 안정 target을 사용하도록 Expo adapter의 성공/실패 event target을
`jamye_server`로 명시한 뒤 같은 card를 재실행한다.

### 기록된 Expo adapter GREEN 증거

2026-08-27 사용자 재실행에서 configuration, request mapping, ticket/HTTP failure
classification과 structured-log redaction test 네 개가 모두 통과했다. Loopback scripted
server만 사용했고 실제 Expo network에는 접근하지 않았으며 46개가 filtered out됐다.
결과는 4 passed, 0 failed 및 `task_9_expo_adapter_green_exit=0`이다.

## Feature-local push configuration RED

```bash
just --justfile scripts/tasks/task-9/mod.just push-config-red
printf 'task_9_push_config_red_exit=%s\n' "$?"
```

이 card는 Task-13의 최종 `.env.example`/NixOS 노출에 앞서 Task-9 worker가 소비할 non-secret
delivery budget과 optional Expo enhanced-security token 경계를 고정한다.

- 기본값은 batch `50`, lease `15s`, provider timeout `2s`, safety margin `1s`, retry
  delay `1s`, poll interval `250ms`, max attempts `8`
- lease는 provider timeout과 safety margin의 합보다 반드시 커야 함
- endpoint는 exact Expo send path의 HTTPS 또는 test용 explicit loopback HTTP만 허용하고
  production은 canonical Expo HTTPS endpoint만 허용
- optional access token과 validation error에는 secret/config value가 노출되지 않음
- 최종 process wiring은 Task-12 소유이므로 `src/bin/api.rs`와 `src/bin/worker.rs`는 변경하지 않음

현재 config는 public shape와 환경 key만 제공하고 입력을 소비하지 않는 compile-valid RED
scaffold다. 유효한 RED는 두 test가 발견된 뒤 default/override 및 invalid-input assertion
차이에서 실패해 exit `101`을 기록하는 것이다. Compile 오류, external Expo 접근 또는 secret
출력은 유효한 RED가 아니다.

### 기록된 feature-local push configuration RED 증거

2026-08-27 사용자 실행에서 production crate와 두 config test가 정상 compile·발견됐다.
Default/override test는 scaffold batch `1`과 계약 default `50`의 assertion 차이로 실패했고,
invalid-input test는 private-LAN endpoint를 거절하지 않아 error key가 `None`인 차이로 실패했다.
결과는 0 passed, 2 failed, 50 filtered out 및 `task_9_push_config_red_exit=101`이며 external
Expo, database 또는 secret 출력은 없었다.

## Feature-local push configuration GREEN

```bash
just --justfile scripts/tasks/task-9/mod.just push-config-green
printf 'task_9_push_config_green_exit=%s\n' "$?"
```

유효한 GREEN은 두 test가 모두 통과하고 exit `0`을 기록하는 것이다. 이 단계는 configuration
value object와 validation만 완성하며 static push runner와 Task-9 aggregate는 후속 gate에서
검증한다.

### 기록된 feature-local push configuration GREEN 증거

2026-08-27 사용자 실행에서 default/override budget 및 redaction test와 invalid-value key
test 두 개가 모두 통과했다. 50개가 filtered out됐고 external Expo, database 또는 secret
출력 없이 `task_9_push_config_green_exit=0`이 기록됐다.

## Static Expo push runtime RED

```bash
just --justfile scripts/tasks/task-9/mod.just push-runtime-red
printf 'task_9_push_runtime_red_exit=%s\n' "$?"
```

이 card는 Task-12의 최종 process composition에 연결하지 않은 채 Task-9이 제공할 정적 Expo
runner 경계를 고정한다.

- validated `AppConfig`와 `PushConfig`만 concrete PostgreSQL repository, transaction manager,
  Expo provider 및 `PushWorker` 조립에 사용
- lazy PostgreSQL pool과 local HTTP client만 생성하고 test 중 database 또는 Expo network에
  접속하지 않음
- polling runtime은 매 cycle 한 번 `run_once`를 수행한 뒤 shutdown 또는 configured interval을
  기다림
- runtime Debug와 composition error에는 database URL, Expo access token, destination 또는
  preview material을 노출하지 않음
- `src/bin/api.rs`와 `src/bin/worker.rs`의 최종 reachability는 Task-12 소유로 유지

현재 static composition은 typed `Worker` error를 반환하고 runtime loop는 shutdown 전에 worker를
poll하지 않는 compile-valid RED scaffold다. 유효한 RED는 두 test가 정상 compile·발견된 뒤
composition 결과와 poll count의 기대 차이에서만 실패해 exit `101`을 기록하는 것이다. Database
connection, Expo network 접근, process entrypoint 변경 또는 secret 출력은 유효한 RED가 아니다.

### 기록된 static Expo push runtime RED 증거

2026-08-27 사용자 실행에서 production crate와 runtime test 두 개가 정상 compile·발견됐다.
Static composition test는 scaffold의 `Err(Worker)`와 기대 `Ok(37ms)` 차이로 실패했고, immediate
shutdown test는 poll count `0`과 기대 `1` 차이로 실패했다. 결과는 0 passed, 2 failed,
52 filtered out 및 `task_9_push_runtime_red_exit=101`이며 database/Expo network 접근, process
entrypoint 변경 또는 secret 출력은 없었다.

## Static Expo push runtime GREEN

```bash
just --justfile scripts/tasks/task-9/mod.just push-runtime-green
printf 'task_9_push_runtime_green_exit=%s\n' "$?"
```

유효한 GREEN은 runtime test 두 개가 모두 통과하고 exit `0`을 기록하는 것이다. GREEN은
feature-local static runner까지만 완성하며 실제 process startup/shutdown 조합은 Task-12에 남긴다.

### 기록된 static Expo push runtime GREEN 1차 시도

2026-08-27 사용자 실행에서 immediate-shutdown polling test는 통과했지만 static composition
test가 SQLx `connect_lazy_with`의 pool maintenance task를 Tokio context 밖에서 생성해
`this functionality requires a Tokio context` panic으로 실패했다. 이는 database connection이나
production composition 실패가 아니라 동기 test harness가 실제 `#[tokio::main]` process 호출
조건을 재현하지 못한 문제다. 결과는 1 passed, 1 failed, 52 filtered out 및
`task_9_push_runtime_green_exit=101`이므로 아직 GREEN이 아니다. Regression fix는 해당
composition test를 `#[tokio::test]`로 실행하는 한 줄이며 lazy pool은 여전히 database에
접속하지 않는다. 이후 SQLx pool을 생성하는 composition test는 lazy 여부와 무관하게 Tokio
context를 명시한다.

### 기록된 static Expo push runtime 최종 GREEN 증거

2026-08-27 사용자 재실행에서 immediate-shutdown polling과 Tokio context 안의 static
composition test 두 개가 모두 통과했다. 52개가 filtered out됐고 database 또는 Expo network
접근 없이 `task_9_push_runtime_green_exit=0`이 기록됐다. 이 결과는 feature-local runner 경계를
닫지만 Task-12가 소유한 실제 process startup/shutdown wiring을 선점하지 않는다.

## Repository quality gates

모든 Task-9 TDD slice가 GREEN인 상태에서 repository-wide formatting과 warning-as-error lint를
각각 검증한다.

```bash
cargo fmt --all -- --check
printf 'task_9_fmt_check_exit=%s\n' "$?"
```

```bash
cargo clippy --locked --all-targets --all-features -- --deny warnings
printf 'task_9_clippy_exit=%s\n' "$?"
```

두 command 모두 `nix develop path:.` 환경에서 실행한다. Formatting gate는 파일을 변경하지 않고
diff 없이 exit `0`이어야 한다. Clippy gate는 repository 전체 target과 feature 조합을 compile해
경고 없이 exit `0`이어야 한다. 실패하면 첫 diagnostic을 보존하고 국소 수정한 뒤 해당 gate부터
재실행한다.

### 기록된 formatting gate 1차 시도

2026-08-27 사용자 실행에서 `cargo fmt --all -- --check`는 compile이나 파일 변경 없이 rustfmt
레이아웃 차이를 보고하고 `task_9_fmt_check_exit=1`로 끝났다. 출력은 Task-9 production/test 파일의
import·호출·함수 시그니처 줄바꿈과 Task-9 constructor 호환 변경이 이미 있는
`tests/realtime_membership/helpers.rs`의 import 줄바꿈 한 건으로 한정됐으며 semantic diagnostic은
없었다. 전체 formatter 적용 후 동일 check를 다시 실행해야 formatting GREEN으로 판정한다.

### 기록된 formatting GREEN 증거

2026-08-27 사용자 실행에서 `cargo fmt --all`이 `task_9_fmt_apply_exit=0`으로 전체 formatter를
적용했고, 이어진 `cargo fmt --all -- --check`가 추가 diff 없이
`task_9_fmt_check_exit=0`을 기록했다.

### 기록된 Clippy gate 1차 시도

2026-08-27 사용자 실행에서 repository-wide Clippy는
`src/adapters/push/expo/mod.rs`의 `valid_endpoint_configuration`에 있는 redundant closure 한 건을
`--deny warnings`로 거절해 `task_9_clippy_exit=101`을 기록했다. `is_ok_and`에 동일한
`valid_endpoint` 함수 포인터를 직접 전달하는 최소 수정으로 해결하며, 이 lint 자체가 closure를
복원하면 다시 실패하는 정적 regression gate이므로 별도 동작 test는 추가하지 않는다. 기존 endpoint
configuration test와 동일 Clippy card를 재실행해 GREEN을 확인한다.

### 기록된 Clippy gate 2차 시도

2026-08-27 사용자 재실행에서 formatting recheck는 `task_9_fmt_recheck_exit=0`을 기록했고,
앞선 redundant-closure 수정은 production crate 검사 단계를 통과했다. 이어 Clippy가 Task-9 test
code의 `expect_used` 세 건을 발견해 전체 gate는 다시 `task_9_clippy_exit=101`로 끝났다. 대상은
bounded preview `Option`, valid test worker construction `Result`, JSON object assertion `Option`이다.
Preview는 `Option` 자체를 assertion하고 나머지 test invariant는 명시적 pattern match로 바꿔
`expect`와 `unwrap` 없이 같은 실패 정보를 유지한다. 이 lint 자체가 세 패턴을 복원하면 실패하는
정적 regression gate이므로 별도 production behavior test는 추가하지 않는다.

### 기록된 repository quality GREEN 증거

2026-08-27 사용자 최종 재실행에서 `cargo fmt --all -- --check`는 추가 diff 없이
`task_9_fmt_recheck_exit=0`을 기록했다. 이어 repository-wide
`cargo clippy --locked --all-targets --all-features -- --deny warnings`가 production 및 test target을
모두 경고 없이 검사해 `task_9_clippy_exit=0`으로 끝났다.

## Task-9 aggregate GREEN

Formatting과 Clippy가 통과한 뒤 다음 카드로 Task-9 전체 notification target과 기존
clean-architecture 경계를 재검증한다.

```bash
just --justfile scripts/tasks/task-9/mod.just green
printf 'task_9_green_exit=%s\n' "$?"
```

카드는 task-1의 exact loopback/test 환경을 읽는다. 먼저 secret-safe structured-log test 한 개를
제외한 notification test 53개를 기존처럼 병렬 실행하고, 해당 log-capture test는 새 Cargo
프로세스에서 직렬 실행한다. 마지막으로 all-feature architecture target을 실행한다. 프로세스 분리는
thread-local capture subscriber와 process-global `tracing` callsite cache 사이의 test-runner 간섭을
차단할 뿐 production 경로나 54개 notification test inventory를 생략하지 않는다. 현재 성공 기준은
합계 notification 54 passed, 0 failed와 architecture target 전체 통과 및 최종 exit `0`이다.

PostgreSQL test는 `JAMYE_ENVIRONMENT=test`와 exact loopback `jamye_test` guard 아래 무작위
`jamye_task_test_*` database만 생성·migration한 뒤 삭제한다. Expo adapter test는 explicit loopback
scripted server만 사용한다. Persistent/production database, external Expo, container, volume, Git state
또는 remote state는 변경하지 않는다.

### 기록된 Task-9 aggregate GREEN 증거

2026-08-27 사용자 실행에서 첫 notification 프로세스는 structured-log test 한 개를 제외한 53개를
모두 통과시켰고, 새 직렬 프로세스는 제외했던 log-capture test 한 개를 통과시켰다. 이어
all-feature architecture target 네 테스트가 모두 통과했다. 합계 결과는 notifications 54 passed,
0 failed, architecture 4 passed, 0 failed 및 `task_9_green_exit=0`이다.
