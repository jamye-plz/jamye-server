# M5b task-6b chatrooms, history, and read cursor cards

## 목적과 범위

Task-6b는 task-6의 group/membership authority와 task-4a의 caller-owned
`TransactionHandle`을 재사용해 다음 경계를 구현한다.

- C1 `GET /api/v1/groups/{group_id}/chatrooms` opaque `after` pagination
- C2 `GET /api/v1/chatrooms/{chatroom_id}/messages` opaque `before` history pagination
- C3 `POST /api/v1/chatrooms/{chatroom_id}/read` server conversation cursor
- sender 정보를 포함한 denormalized text history와 task-8 이전의 정확한 `media: []`
- `(user_id, chatroom_id)`별 하나의 monotonic canonical read marker
- exact `0003` predecessor에서 forward-only `0004_chatroom_reads.sql` upgrade/rollback

`conversation_events`는 delta/recovery correctness log로 유지하며 C2 message history를
대체하지 않는다. Task-9의 bounded notification clearing과 task-12의 최종
`MarkConversationRead` composition은 후속 범위다.

## 선행 조건

- task-6 GREEN과 guarded Redis recovery evidence가 완료돼 있어야 한다.
- `nix develop path:.` devShell 안이어야 한다.
- RED 실행 시점에는 `migrations/0004_chatroom_reads.sql`과 task-6b production surface가
  아직 없어야 한다.
- RED에는 PostgreSQL, Redis, provider credential, API/worker 또는 container lifecycle이
  필요하지 않는다. GREEN에는 guarded local PostgreSQL이 healthy여야 한다.

## RED

```bash
just --justfile scripts/tasks/task-6b/mod.just red
printf 'task_6b_red_exit=%s\n' "$?"
```

유효한 RED는 test binary가 현재 locked graph에서 정상 compile된 뒤 다음 원인으로
nonzero 종료한다.

```text
RED: migrations/0004_chatroom_reads.sql is absent; task-6b must add chatroom queries, denormalized message history, monotonic read cursors, and static C1-C3 composition
```

예상 종료 코드는 Cargo test failure인 `101`이다. Compile 오류, lockfile 변경, 다른
assertion 실패, 또는 migration이 이미 존재해 card가 exit `2`로 거절한 결과는 유효한
RED가 아니다. Raw output을 전달한 뒤에만 migration, behavior tests와 production
implementation을 작성한다.

### 기록된 RED evidence

2026-08-26 사용자 실행에서 locked graph가 정상 compile되고 단일 absence test가
`migrations/0004_chatroom_reads.sql` 부재를 정확히 보고한 뒤 Cargo test exit `101`로
종료했다. 따라서 이후 RED recipe는 migration이 존재하는 GREEN tree에서 재실행하지 않는다.

## 부작용과 복구

- Cargo compile/test cache만 만들 수 있다.
- Source, migration, lockfile, database, Redis key, container와 Git state는 변경하지 않는다.
- 실패 시 첫 compile/error/assertion을 그대로 보존한다. RED를 재현하려고 task-6 source,
  migration 또는 shared volume을 삭제하지 않는다.

## RED 이후 경계

Task-6b는 새 Cargo dependency를 소유하지 않는다. 기존 SQLx/Axum/auth/transaction 경계를
재사용하며, 새 dependency가 필요하다고 판명되면 `Cargo.toml`이나 lockfile을 수정하기
전에 dependency-plan 교정을 먼저 제시한다.

유효한 RED 뒤에는 exact `0003` migration upgrade/rollback과 read-marker repository를
먼저 GREEN으로 만든다. 그 뒤 membership/BOLA, C1/C2 page boundary, denormalized sender와
`media: []`, stale/duplicate/concurrent C3를 순서대로 추가한다.

준비된 GREEN surface는 다음을 함께 검증한다.

- exact `0003` → `0004` upgrade와 강제 실패 시 relation 전체 rollback
- 한 SQL snapshot 안의 membership/BOLA 판정과 C1/C2 cursor anchor 검증
- C2 page 내부 chronological ordering, sender projection, `media: []`, event-log 독립성
- caller-owned transaction을 받는 C3 base operation과 standalone wrapper
- unknown/cross-conversation 무변경, stale/duplicate idempotency, concurrent maximum convergence
- C1–C3 DTO/schema/fixture contribution과 architecture 경계

## GREEN

```bash
just --justfile scripts/tasks/task-6b/mod.just green
printf 'task_6b_green_exit=%s\n' "$?"
```

이 카드는 task-6b의 migration, application/PostgreSQL/HTTP/contract tests와 기존
architecture target을 실행한다. GREEN 명령은 유효한 RED evidence를 받은 뒤 production
구현이 준비됐을 때만 실행한다.

### 기록된 GREEN evidence

2026-08-26 사용자 실행에서 task-6b chatrooms target `13/13`과 기존 architecture target
`4/4`가 모두 통과했고 `task_6b_green_exit=0`을 확인했다. 출력에 남은 두 경고는 기존
`SensitiveValue` 필드와 `expose_secret` 메서드의 `dead_code` 경고이며 task-6b 실패나
chatroom 동작 결함은 아니다.
