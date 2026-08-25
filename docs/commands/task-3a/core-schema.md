# M2 task-3a core schema card

## 목적

D1=A(no-pruning v1)를 PostgreSQL core schema에 materialize하기 전에 테스트가 올바른 이유로 실패하는지 확인하고, 구현 뒤에는 SQLx `0001` migration의 원자성·제약·server cursor·outbox 초기 상태를 검증한다.

## RED

선행 조건은 pinned Nix devShell뿐이다. PostgreSQL을 시작하지 않으며 `migrations/0001_core_reliable_messaging.sql`이 아직 없어야 한다.

```bash
nix develop path:. --command just --justfile scripts/tasks/task-3a/mod.just red
printf 'task_3a_red_exit=%s\n' "$?"
```

성공적인 RED 증거는 test가 `RED: migrations/0001_core_reliable_messaging.sql is absent`를 보고하고 nonzero로 끝나는 것이다. compile 오류나 다른 test 실패는 유효한 RED가 아니다. raw output을 에이전트에게 전달한 뒤에만 migration과 ADR 구현을 진행한다.

## GREEN

RED가 확인되고 migration/ADR이 작성된 뒤에만 실행한다. task-1 local environment가 존재하고 disposable PostgreSQL이 healthy여야 한다.

```bash
nix develop path:. --command just task-1 infra-up
nix develop path:. --command just --justfile scripts/tasks/task-3a/mod.just green
printf 'task_3a_green_exit=%s\n' "$?"
```

## 부작용

- RED는 Cargo compile/test cache만 만들 수 있다. source, lockfile, service, Git state를 변경하지 않는다.
- GREEN은 `.env.local`의 exact `JAMYE_ENVIRONMENT=test`, loopback PostgreSQL, `/jamye_test`만 허용한다.
- GREEN은 `jamye_task_3a_<uuid>` 이름의 database를 test마다 만들고 종료 시 `DROP DATABASE ... WITH (FORCE)`로 그 exact database만 제거한다.
- migration은 local disposable database에만 적용한다. persistent/production database, legacy repository, Podman volume을 변경하지 않는다.

## 성공 기준

- RED: migration 부재 assertion 때문에 nonzero.
- GREEN: `tests/core_schema.rs` 전체가 exit `0`.
- GREEN은 빈 즉시-prior 상태에서 0001 적용, 정확히 7개 core table, partial uniqueness/check constraints, server-generated monotonic cursor, outbox default/shape, forced migration failure 시 partial table 0개를 증명한다.

## 복구

RED가 예상과 다른 이유로 실패하면 migration을 작성하지 말고 raw diagnostic을 보존한다. GREEN 실패 시 첫 diagnostic과 `just task-1 infra-status` 출력을 전달한다. test process가 강제 종료돼 prefix database가 남았다고 의심되면 임의 삭제하지 말고 database 이름을 먼저 확인해 exact guarded cleanup 절차를 별도로 결정한다. `infra-reset`은 이 test의 일반 복구 명령이 아니다.
