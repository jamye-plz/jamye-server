# M2b task-3c dev identity and fixture harness card

## 목적

C1이 production OAuth와 전체 group CRUD를 기다리지 않고 실제 Bearer identity와
chatroom으로 검증될 수 있도록, compile-time `dev-fixtures` feature와 runtime
`JAMYE_ENABLE_DEV_FIXTURES` guard를 모두 요구하는 non-production seed endpoint를
구현한다. default production composition에는 route, issuer, signing key, acceptance
path가 없어야 한다.

## RED

선행 조건은 pinned Nix devShell뿐이다. PostgreSQL이나 다른 local service를
시작하지 않으며 `src/dev_fixtures/mod.rs`가 아직 없어야 한다.

```bash
just --justfile scripts/tasks/task-3c/mod.just red
printf 'task_3c_red_exit=%s\n' "$?"
```

유효한 RED는 test binary가 정상 compile된 뒤
`RED: src/dev_fixtures/mod.rs is absent`를 보고하고 nonzero로 끝나는 것이다.
compile 오류나 다른 assertion 실패는 유효한 RED가 아니다. raw output을 전달한
뒤에만 auth verifier, Axum extractor, atomic seed endpoint를 구현한다.

## 부작용

- Cargo compile/test cache만 만들 수 있다.
- source, lockfile, service, database, Git state를 변경하지 않는다.
- dev signing material이나 fixture data를 생성하지 않는다.

## 성공 기준

- test target이 정상 compile된다.
- 오직 task-3c 구현 surface 부재 assertion 때문에 실패한다.
- default production exclusion을 검사할 최종 regression target과 같은
  `tests/dev_fixtures.rs`를 사용한다.

## 복구

예상과 다른 이유로 실패하면 구현을 시작하지 말고 첫 diagnostic 전체를
보존해 전달한다. `src/dev_fixtures/mod.rs`가 이미 존재한다면 RED를 재현하려고
파일을 삭제하거나 Git 상태를 되돌리지 않는다.

## 승인된 dependency 경계

RED 이후 사용자는 A안을 승인했다. `Cargo.toml`은 `default=[]`를 유지하고
`dev-fixtures=["dep:jsonwebtoken"]`만 연결한다. `jsonwebtoken 11.0.0`은 optional,
`default-features=false`, `rust_crypto` feature로 한정한다. 따라서 default build는
JWT crate, dev issuer, in-memory key, seed route를 만들지 않는다. exact Rust release는
계속 `rust-toolchain.toml` 하나만 소유한다.

구현은 SQLx를 `src/adapters/postgres/` 밖으로 내보내지 않는다. endpoint는 얇은
transport이고, dev service가 port를 호출하며 PostgreSQL adapter가 user/group/owner
membership/main chatroom 네 insert를 transaction 하나로 commit한다.

## Lock 및 dependency 검증

`Cargo.lock`은 agent가 손으로 편집하지 않는다. 다음 card를 순서대로 실행한다.

```bash
just --justfile scripts/tasks/task-3c/mod.just locks
printf 'task_3c_locks_exit=%s\n' "$?"

just --justfile scripts/tasks/task-3c/mod.just dependency-check
printf 'task_3c_dependency_exit=%s\n' "$?"
```

`locks`는 task-1의 canonical lock 생성/no-drift primitive를 재사용한다. 성공 시
Cargo.lock과 flake.lock checksum, no-drift 문구와 exit `0`이 출력되어야 한다.
dependency card는 advisory, ban, license, source가 모두 통과해야 한다. 실패하면
finding 전체를 전달하고 dependency 예외를 임의로 추가하지 않는다.

## GREEN

선행 조건은 healthy local PostgreSQL과 task-1이 만든 mode-0600 `.env.local`이다.
card는 exact test environment와 loopback `/jamye_test`만 허용하고 매 test마다
별도 database를 만들고 제거한다.

```bash
just --justfile scripts/tasks/task-3c/mod.just green
printf 'task_3c_green_exit=%s\n' "$?"
```

GREEN은 다음을 모두 검증한다.

- runtime env만 켠 default-feature target에는 dev route/issuer/key/JWT dependency가 없다.
- feature+runtime guard에서 실제 `POST /__dev/fixtures/seed`가 201을 반환한다.
- 반환 UUID 네 개가 한 transaction으로 저장되고 owner/main 관계가 일치한다.
- Bearer는 UUID `sub`, signed UUID `sid`, `iss=jamye-dev`, `aud=jamye-api`, 5분 `exp`를 가진다.
- missing/malformed/expired/wrong issuer·audience/non-UUID sub·sid/signature mismatch가 동일한
  401 `authentication_required` envelope로 거절된다.
- shared direct-row helpers와 SQLx adapter confinement architecture test가 통과한다.

## GREEN 복구

lock error면 먼저 `locks`를 다시 확인한다. runtime guard error면 `.env.local`의
`JAMYE_ENVIRONMENT=test`를 확인하되 값을 출력하지 않는다. database error면
`just task-1 infra-status` 결과를 전달한다. 실패한 disposable database는
`jamye_task_test_` allowlist 밖의 database를 삭제하는 근거가 되지 않는다.
