# M3a task-4a reliable messaging RED card

## 목적

task-3a가 고정한 D1=A PostgreSQL schema와 task-3b가 고정한 C0/D8=A wire를
소비해 message command, delta, 그리고 단일 caller-owned transaction 경계를
구현하기 전에 같은 최종 `tests/messaging.rs` target이 올바른 이유로 실패하는지
확인한다.

이 단계는 DTO, repository trait, SQLx transaction API를 아직 만들지 않는다.
task-4a 구현 surface가 없다는 사실만 증명하므로 RED 결과를 보고 구현 계약을
자의적으로 바꾸지 않는다.

## 선행 조건

- task-3a, task-3b, task-3c가 완료돼 있어야 한다.
- `nix develop path:.` devShell 안이어야 한다.
- `src/domain/messaging/mod.rs`가 없어야 한다.
- PostgreSQL, Redis, MinIO, API, worker는 필요하지 않다.

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
exit `2`로 거절한 결과는 유효한 RED가 아니다. raw output을 전달한 뒤에만
`tests/messaging.rs`를 frozen C0 wire 및 실제 PostgreSQL/Axum 통합 사례로 확장하고
production 구현을 시작한다.

## 부작용

- Cargo compile/test cache만 만들 수 있다.
- source, lockfile, service, database, Podman, Git state를 변경하지 않는다.
- 인증 token이나 fixture row를 만들지 않는다.

## 복구

예상 문구 외의 이유로 실패하면 구현을 시작하지 말고 첫 diagnostic 전체를
보존한다. messaging source가 이미 있다면 RED를 재현하려고 파일을 삭제하거나
Git 상태를 되돌리지 않는다.

## 이후 GREEN 경계

RED 확인 뒤 동일 target은 C0의 정확한 text-only C4/S1 DTO, body-authoritative
`client_msg_id`, optional matching header, same-payload canonical retry, changed-payload
zero-mutation conflict, message/event/outbox 한 transaction, rollback, pagination,
unknown-event marker, dev Bearer, PostgreSQL outage/recovery, safe logging을 검증하도록
확장한다. task-4a가 sole `TransactionHandle`/SQLx manager를 소유하고 repository는
caller-owned handle을 소비할 뿐 begin/commit하지 않는다. 실행 가능한 GREEN 및
PostgreSQL recovery card는 그 구현과 함께 이 task-owned Just module에 추가한다.
