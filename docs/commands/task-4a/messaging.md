# M3a task-4a reliable messaging cards

## 목적

task-3a가 고정한 D1=A PostgreSQL schema와 task-3b가 고정한 C0/D8=A wire를
소비해 message command, delta, 그리고 단일 caller-owned transaction 경계를
검증한다. RED는 최종 target이 구현 surface 부재로만 실패했음을 증명했고,
GREEN 두 개는 정상 동작과 PostgreSQL 장애/복구를 분리해 검증한다.

## 선행 조건

- task-3a, task-3b, task-3c가 완료돼 있어야 한다.
- `nix develop path:.` devShell 안이어야 한다.
- RED에는 PostgreSQL, Redis, MinIO, API, worker가 필요하지 않다.
- GREEN에는 task-1의 mode-0600 `.env.local`과 healthy local PostgreSQL이 필요하다.
- Redis와 MinIO는 task-4a 요청 경로에 필요하지 않으며 card가 시작하거나 중지하지 않는다.

## RED

```bash
just --justfile scripts/tasks/task-4a/mod.just red
printf 'task_4a_red_exit=%s\n' "$?"
```

유효한 RED는 test binary가 정상 compile된 뒤 다음 원인으로 nonzero 종료한다.

```text
RED: src/domain/messaging/mod.rs is absent; task-4a must add the reliable messaging domain, transaction, PostgreSQL, and HTTP surfaces
```

compile 오류, lockfile 변경, 다른 assertion 실패, 또는 source가 이미 존재해 card가
exit `2`로 거절한 결과는 유효한 RED가 아니다. 2026-08-25 사용자 실행에서는
예상한 surface 부재 한 건으로 실패했고 `task_4a_red_exit=101`이 확인됐다.

## GREEN

```bash
just --justfile scripts/tasks/task-4a/mod.just green
printf 'task_4a_green_exit=%s\n' "$?"
```

이 카드는 다음을 검증한다.

- body-authoritative `client_msg_id`, optional matching header, D8=A 201/200/409 의미
- missing/null/empty 본문, whitespace 보존, C1 `media_not_available`
- 실제 dev Bearer extractor와 membership/BOLA
- concurrent retry에도 message/event/outbox 각 한 row
- event insert 실패 시 전체 rollback
- current/previous version delta, `>2*limit`, terminal empty page, between-page commit,
  safe unknown marker와 unsafe projection 426
- application만 한 handle을 begin/commit하고 repository는 같은 handle을 소비하는 구조
- SQLx adapter confinement 및 runtime registry 부재

## PostgreSQL recovery와 안전 로그

일반 GREEN이 exit 0인 뒤 실행한다.

```bash
just --justfile scripts/tasks/task-4a/mod.just postgres-recovery
printf 'task_4a_recovery_exit=%s\n' "$?"
```

이 card는 두 단계다. 먼저 연결 불가능한 별도 loopback endpoint를 주입해 C4/S1의
safe 503과 structured-log redaction을 빠르게 검증한다. 다음으로 random disposable
database와 한 개의 in-process Axum Router를 만든 뒤, 정확히 guarded local Compose
project의 PostgreSQL service만 실제로 중지한다. 같은 test/API process와 Router가
살아 있는 동안 `/health/live=200`, `/health/ready=503`, C4/S1의
`database_unavailable`을 확인한다. PostgreSQL을 같은 volume으로 다시 시작한 뒤 같은
Router의 readiness가 회복되고 partial row가 0개임을 확인한 다음, outage 때 사용한
동일 `client_msg_id`를 재시도해 message/event/outbox가 각각 한 row만 commit되는지
검증한다.

다른 local 작업이 `jamye-server-test` PostgreSQL을 사용 중이면 먼저 종료해야 한다.
card는 Redis와 MinIO를 건드리지 않고 PostgreSQL named volume을 삭제하거나 초기화하지
않는다. 실패·SIGINT·SIGTERM 시 trap이 PostgreSQL을 다시 시작하고 health를 기다린다.
2026-08-25 사용자 실행에서는 빠른 recovery/log 테스트 2/2와 같은 Router를 유지한
실제 stop/start 테스트 1/1이 통과했고 `task_4a_postgres_recovery_exit=0`을 확인했다.

## 부작용과 복구

- RED는 Cargo compile/test cache만 만들 수 있다.
- 두 GREEN card는 `jamye_test` 아래 random disposable database만 만들고 종료 시 제거한다.
- recovery card는 guarded `jamye-server-test` PostgreSQL container를 stop/start하지만
  named-volume bytes를 보존한다. persistent/production database, Redis, MinIO, lockfile,
  source, Git state는 변경하지 않는다.
- database 연결 실패면 첫 diagnostic과 `just task-1 infra-status`를 전달한다.
- behavior/architecture 실패면 다음 card로 넘어가지 않고 첫 실패를 그대로 전달한다.
- 중단되었는데 PostgreSQL이 자동 복구되지 않으면 `just task-1 infra-up`을 실행하고
  `just task-1 infra-status` 결과를 확인한다. volume reset은 하지 않는다.

## 구현 경계

동일 target은 C0의 정확한 text-only C4/S1 DTO, body-authoritative `client_msg_id`,
optional matching header, same-payload canonical retry, changed-payload zero-mutation
conflict, message/event/outbox 한 transaction, rollback, pagination, unknown-event marker,
dev Bearer, PostgreSQL outage/recovery, safe logging을 검증한다. task-4a가 sole
`TransactionHandle`/SQLx manager를 소유하고 repository는 caller-owned handle을
소비할 뿐 begin/commit하지 않는다. task-4b는 이 ordinary static Router와 handle을
직접 소비하며 task-12는 새 transaction abstraction을 만들지 않는다.
