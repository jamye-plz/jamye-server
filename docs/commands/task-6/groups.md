# M5 task-6 groups, memberships, roles, and invites cards

## 목적과 범위

Task-6는 task-4a의 단일 caller-owned `TransactionHandle`과 task-5의 shared Redis
rate-limit adapter를 재사용해 다음 경계를 구현한다.

- G1-G8 그룹 생성·조회·수정·soft-delete·멤버 관리·소유권 이전
- I1/I2 bounded invite 발급·가입과 동일 사용자/전역 사용량 동시성 fence
- 그룹·owner membership·정확히 하나의 main chatroom을 한 transaction에서 생성
- live group → membership/invite 순서의 PostgreSQL lock과 기본 member cap 12
- exact `0002` predecessor에서 forward-only `0003_invites.sql` upgrade/rollback

RED 뒤에는 production source와 migration을 추가하고, 같은 task-owned target으로 GREEN과
Redis stop/restart recovery를 증명한다.

## 선행 조건

- task-4a와 task-5의 GREEN/recovery evidence가 완료돼 있어야 한다.
- D6=A configurable conservative rate-limit evidence는 task-5에서 고정돼 있어야 한다.
- `nix develop path:.` devShell 안이어야 한다.
- RED 실행 시점에는 `migrations/0003_invites.sql`과 task-6 production surface가 아직
  없어야 한다.
- RED에는 PostgreSQL, Redis, provider credential, API/worker 또는 Podman lifecycle이
  필요하지 않다. GREEN/recovery에는 guarded local PostgreSQL과 Redis가 healthy여야 한다.

## RED

```bash
just --justfile scripts/tasks/task-6/mod.just red
printf 'task_6_red_exit=%s\n' "$?"
```

유효한 RED는 test binary가 현재 locked graph에서 정상 compile된 뒤 다음 원인으로
nonzero 종료한다.

```text
RED: migrations/0003_invites.sql is absent; task-6 must add group topology, memberships, roles, bounded invites, and shared rate-limit integration
```

2026-08-26 사용자 실행은 현재 locked graph compile 뒤 정확히 위 migration 부재
문구로 실패했고 `task_6_red_exit=101`을 반환했다. 따라서 task-6 TDD RED evidence는
유효하다.

예상 종료 코드는 Cargo test failure인 `101`이다. Compile 오류, lockfile 변경, 다른
assertion 실패, 또는 migration이 이미 존재해 card가 exit `2`로 거절한 결과는 유효한
RED가 아니다. Raw output을 전달한 뒤에만 migration, behavior tests와 production
implementation을 작성한다.

## 부작용과 복구

- Cargo compile/test cache만 만들 수 있다.
- Source, migration, lockfile, database, Redis key, container와 Git state는 변경하지 않는다.
- 실패 시 첫 compile/error/assertion을 그대로 보존한다. RED를 재현하려고 기존 task-5
  source나 migration을 삭제하거나 shared volume을 reset하지 않는다.

## RED 이후 경계

Task-6은 새 Cargo dependency를 소유하지 않는다. 기존 SQLx/Axum/auth/rate-limit 경계를
재사용하며, 새 dependency가 실제로 필요하다고 판명되면 `Cargo.toml`이나 lockfile을
수정하기 전에 dependency-plan 교정을 먼저 제시한다.

유효한 RED 뒤에는 그룹 topology transaction과 rollback을 먼저 GREEN으로 만들고, 이후
ownership/invite concurrency, HTTP/contract contribution, Redis outage/recovery를 순서대로
추가한다. Task-6c의 realtime revocation control intent는 아직 후속 범위다.

## GREEN

```bash
just --justfile scripts/tasks/task-6/mod.just green
printf 'task_6_green_exit=%s\n' "$?"
```

이 카드는 로컬 환경을 불러온 뒤 다음을 한 번에 검증한다.

- exact `0002` → `0003` upgrade와 강제 실패 시 `invites` relation 전체 rollback
- G1의 group → owner membership → main chatroom 단일 transaction과 세 insertion-point rollback
- main chatroom partial unique constraint와 소유권 이전 atomicity/409 conflict
- membership-gated 조회, keyset pagination, soft-delete 404
- 기존 멤버 우선 I2, 동일 사용자·전역 `max_uses`·그룹 삭제 경합, 기본 정원 12
- G1-G8/I1-I2 HTTP status/body/error envelope와 task-6 contract contribution
- task-5 shared Redis fixed-window namespace의 allowance/429/reset/actor-IP isolation
- application/port/PostgreSQL/HTTP 정적 경계와 repository의 begin/commit/rollback 부재
- 기존 architecture target

성공 기준은 모든 target이 통과하고 마지막 줄이 `task_6_green_exit=0`인 것이다. 경고만
있고 test failure가 없으면 통과지만, 첫 compile 오류나 첫 assertion 실패가 있으면 전체
출력을 그대로 반환한다.

2026-08-26 사용자 실행은 group target 22개와 architecture target 4개를 모두 통과했고
`task_6_green_exit=0`을 반환했다. `SensitiveValue` 관련 두 dead-code warning은 task-12의
최종 production composition 전까지 task-5 비밀값 accessor가 소비되지 않아 발생하며,
task-6 GREEN 실패가 아니다.

## Redis stop/restart recovery

```bash
just --justfile scripts/tasks/task-6/mod.just redis-recovery
printf 'task_6_redis_recovery_exit=%s\n' "$?"
```

이 카드는 guarded local Redis만 중지한다. 같은 `RedisRateLimiter`와 같은 application
service를 유지한 채 outage 중 invite issue/redeem이 stable 503 원인으로 fail closed하고,
PostgreSQL의 invite row·`used_count`·membership이 변하지 않는지 확인한다. 같은 Redis를
재시작한 뒤 기존 invite로 가입이 정상 복구되는지도 증명한다. PostgreSQL container와
named volume은 건드리지 않는다.

성공 기준은 다음 문구와 exit `0`이다.

```text
same Redis limiter recovered; PostgreSQL invite and membership authority was preserved
```

2026-08-26 사용자 실행은 guarded local Redis만 중지·재시작한 뒤 같은 limiter가 복구되고
PostgreSQL invite·`used_count`·membership authority가 보존됨을 확인했다. Recovery test 1개가
통과했고 `task_6_redis_recovery_exit=0`을 반환했다.

## 실패와 복구

- GREEN 실패 시 테스트가 만든 데이터베이스는 disposable `jamye_task_test_*` 범위이며
  정상 경로에서 강제 삭제된다. 실패한 database 이름과 첫 오류를 보존하고 shared
  `jamye_test`를 수동 reset하지 않는다.
- Redis recovery script는 interrupt/error trap에서 중지한 Redis를 다시 시작한다.
  PostgreSQL/volume을 삭제하지 않으며, 복구 문구가 없으면 `compose ps`와 첫 Rust 오류를
  함께 확인한다.
- 두 카드 모두 source, migration, lockfile 또는 Git state를 변경하지 않는다. Task-6은
  새 Cargo dependency를 추가하지 않았으므로 locks/dependency-check 재생성 gate가 없다.
