# M5c task-6c realtime membership revocation card

## 목적과 범위

Task-6c는 task-6의 authoritative membership/group mutation과 task-4b의 multi-node realtime
delivery를 연결한다. 새 테이블이나 migration을 만들지 않고 기존 `outbox_events`에 내부용
typed control intent를 같은 PostgreSQL transaction으로 기록한다.

- member removal과 voluntary leave는 `membership.revoked` control intent를 함께 commit한다.
- group soft-delete는 `group.deleted` control intent를 함께 commit한다.
- 각 API node는 Redis control을 받으면 affected subscription을 먼저 제거한 뒤 WebSocket을
  code `4001`, reason `membership_revoked|group_deleted`로 닫는다.
- Redis Pub/Sub 신호를 놓친 node도 최종 event delivery 전에 authoritative membership을
  batch 검증해 fail closed한다.
- control payload는 내부 조정 신호이며 public realtime event union에 추가하지 않는다.

Task-12가 최종 API/worker composition을 소유한다. Task-6c는 task-6의 cohesive mutation
boundary와 task-4b의 concrete realtime capability를 직접 호출하는 ordinary static operation과
local runner만 추가하며 기존 groups/realtime port를 변경하거나 shared port, plugin registry,
병렬 UnitOfWork를 만들지 않는다.

## 선행 조건

- task-4b GREEN과 guarded Redis recovery가 완료돼 있어야 한다.
- task-6 GREEN과 guarded Redis recovery가 완료돼 있어야 한다.
- `nix develop path:.` devShell 안이어야 한다.
- RED 실행 시점에는 `src/application/realtime/membership_revocation/mod.rs`가 없어야 한다.
- RED에는 PostgreSQL, Redis, API/worker 또는 container lifecycle이 필요하지 않는다.

## RED

```bash
just --justfile scripts/tasks/task-6c/mod.just red
printf 'task_6c_red_exit=%s\n' "$?"
```

유효한 RED는 locked graph와 `tests/realtime_membership.rs`가 정상 compile된 뒤 다음 원인으로
nonzero 종료한다.

```text
RED: src/application/realtime/membership_revocation/mod.rs is absent; task-6c must add atomic membership control intents, multi-node Redis eviction, and fail-closed delivery authorization
```

예상 종료 코드는 Cargo test failure인 `101`이다. Compile 오류, lockfile 변경, 다른 assertion
실패, 또는 production sentinel이 이미 존재해 card가 exit `2`로 거절한 결과는 유효한 RED가
아니다. Raw output을 전달한 뒤에만 behavior tests와 production implementation을 작성한다.

### 기록된 RED 증거 — 2026-08-26

- locked graph와 `tests/realtime_membership.rs`가 정상 compile됐다.
- 의도한 production sentinel 부재 assertion만 실패했다.
- 종료 코드는 `task_6c_red_exit=101`이었다.
- `SensitiveValue` 관련 두 `dead_code` warning은 task-5부터 남아 있는 비차단 경고이며 RED
  판정과 무관하다.

## 부작용과 복구

- Cargo compile/test cache만 만들 수 있다.
- Source, migration, lockfile, PostgreSQL, Redis key, container와 Git state는 변경하지 않는다.
- 실패 출력은 첫 compile/error/assertion부터 그대로 보존한다.
- RED를 재현하려고 task-4b/task-6 source, database row, Redis state 또는 volume을 삭제하지
  않는다.

## RED 이후 구현 경계

Task-6c는 새 Cargo dependency나 migration을 소유하지 않는다. 필요한 dependency가 새로
발견되면 `Cargo.toml`이나 lockfile을 수정하기 전에 dependency-plan 교정을 먼저 제시한다.

유효한 RED 뒤에는 다음 순서로 GREEN을 만든다.

1. task-6의 실제 remove/leave/delete operation과 typed control-intent insert를 하나의
   caller-owned `TransactionHandle`에서 실행하고 cumulative rollback을 증명한다.
2. 기존 typed outbox schema를 claim하는 closed task-6c control worker와 Redis internal control
   channel을 추가한다.
3. 각 local registry에서 unsubscribe를 close보다 먼저 수행하고 모든 affected socket을
   exact `4001` reason으로 종료한다.
4. delivery recipient를 한 번에 검증하는 PostgreSQL batch query로 N+1을 피하고 uncertainty,
   database error, deleted group과 revoked membership을 모두 fail closed한다.
5. two-node signal delivery, intentionally dropped Redis signal, broadcast/revocation race와 public
   event-union 비노출을 검증한다.

## GREEN

GREEN 구현이 준비된 뒤 사용자가 다음 card를 실행한다. Guarded local PostgreSQL과 Redis가
healthy여야 한다.

```bash
just --justfile scripts/tasks/task-6c/mod.just green
printf 'task_6c_green_exit=%s\n' "$?"
```

이 카드는 task-6c transaction, multi-node registry/control, final-delivery authorization,
contract/security tests와 기존 architecture target을 실행한다.

### 기록된 GREEN 증거 — 2026-08-26

- `realtime_membership` 테스트 9개가 모두 통과했다.
- 기존 architecture 테스트 4개가 모두 통과했다.
- 종료 코드는 `task_6c_green_exit=0`이었다.
- `SensitiveValue`와 일부 test-support helper의 `dead_code` warning은 비차단 경고이며 GREEN
  판정과 무관하다.

Task-12 전까지 최종 API/worker composition에는 연결하지 않는다.
