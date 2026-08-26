# M6 task-7 topics, tags, unread, and announcement cards

## 목적과 범위

Task-7은 task-6의 live group/membership authority, task-6b의 monotonic
`chatroom_reads.last_read_cursor`, task-4a의 caller-owned `TransactionHandle`을 재사용해
다음 경계를 구현한다.

- T1–T7 topic lifecycle, timeline/date query, author-only patch와 author|owner tag replacement
- topic, topic chatroom, `topic.created` event/outbox, author read marker를 만드는 원자적 T1
- main chatroom의 작성자 귀속 announcement message와 별도 `message.created` event/outbox
- topic 수와 무관한 O(1) query 수의 server-cursor unread projection
- title-only seed와 body PATCH의 idempotent enriched promotion
- exact `0004` predecessor에서 `topics`, base `topic_media`, `topic_tags`를 만든 뒤
  `chatrooms.topic_id` FK를 추가하는 forward-only `0005_topics.sql`

Task-8의 photo-finalize promotion, task-9 notification/push occurrence, task-12의 최종
`CreateTopic` composition은 후속 범위다. 첫 topic-chat message에 별도 system message를
추가하지 않는다.

## 선행 조건

- task-6 GREEN과 guarded Redis recovery가 완료돼 있어야 한다.
- task-6b GREEN이 완료돼 있어야 한다.
- `nix develop path:.` devShell 안이어야 한다.
- RED 실행 시점에는 `migrations/0005_topics.sql`과 task-7 production surface가 없어야 한다.
- RED에는 PostgreSQL, Redis, API/worker 또는 container lifecycle이 필요하지 않는다.

## RED

```bash
just --justfile scripts/tasks/task-7/mod.just red
printf 'task_7_red_exit=%s\n' "$?"
```

유효한 RED는 `tests/topics.rs`가 현재 locked graph에서 정상 compile된 뒤 다음 원인으로
nonzero 종료한다.

```text
RED: migrations/0005_topics.sql is absent; task-7 must add topic lifecycle, tags, cursor-based unread projection, atomic announcements, and static T1-T7 composition
```

예상 종료 코드는 Cargo test failure인 `101`이다. Compile 오류, lockfile 변경, 다른
assertion 실패, 또는 migration이 이미 존재해 card가 exit `2`로 거절한 결과는 유효한
RED가 아니다. Raw output을 전달한 뒤에만 migration, behavior tests와 production
implementation을 작성한다.

### 기록된 RED 증거

2026-08-26 사용자 실행에서 `tests/topics.rs`가 정상 compile된 뒤 위의 migration-absence
문구로 실패했고 `task_7_red_exit=101`이 기록됐다. 따라서 task-7 구현 게이트는 열렸으며,
현재 production 구현과 회귀 테스트가 준비돼 GREEN 사용자 실행만 남아 있다.

## 부작용과 복구

- Cargo compile/test cache만 만들 수 있다.
- Source, migration, lockfile, PostgreSQL, Redis key, container와 Git state는 변경하지 않는다.
- 실패 출력은 첫 compile/error/assertion부터 그대로 보존한다.
- RED를 재현하려고 기존 migration, task-6/task-6b source 또는 shared volume을 삭제하지 않는다.

## RED 이후 구현 경계

Task-7은 새 Cargo dependency를 소유하지 않는다. 기존 SQLx/Axum/auth/transaction 경계를
재사용하며, 새 dependency가 필요하다고 판명되면 `Cargo.toml`이나 lockfile을 수정하기 전에
dependency-plan 교정을 먼저 제시한다.

유효한 RED 뒤에는 exact `0004` → `0005` upgrade와 forced-failure rollback, live-group lock을
유지하는 T1 topology transaction, same-key idempotency, distinct topic/announcement events를
먼저 GREEN으로 만든다. 이후 author-only patch, author|owner tags, cursor unread와 timezone
boundary, T1–T7 HTTP/contract contribution을 추가한다.

## GREEN

production 구현이 준비된 뒤 사용자가 다음 카드를 실행한다.

```bash
just --justfile scripts/tasks/task-7/mod.just green
printf 'task_7_green_exit=%s\n' "$?"
```

이 카드는 task-7 migration/application/PostgreSQL/HTTP/contract target과 기존 architecture
target을 실행한다. GREEN 성공 여부는 사용자 실행 결과로만 판정한다.

### 기록된 GREEN 증거

2026-08-26 사용자 실행에서 task-7 topics target 16/16과 architecture target 4/4가 모두
통과했고 `task_7_green_exit=0`이 기록됐다. 출력의 `SensitiveValue` 및
`expose_secret` dead-code 경고는 task-5부터 존재한 선행 경고이며 task-7 실패가 아니다.
