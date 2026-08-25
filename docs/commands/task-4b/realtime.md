# M3b task-4b realtime C1 cards

## 목적

task-4a가 고정한 PostgreSQL message/event/outbox 및 delta correctness path 위에
outbox delivery, Redis Pub/Sub, one-time realtime ticket, authorized WebSocket, static
C1 api/worker composition을 추가한다. RED는 최종 target이 realtime 구현 surface
부재로만 실패하는지 먼저 증명한다.

## 선행 조건

- task-4a의 GREEN, architecture, injected recovery/log, 실제 PostgreSQL stop/start
  evidence가 모두 완료돼 있어야 한다.
- `nix develop path:.` devShell 안이어야 한다.
- RED에는 PostgreSQL, Redis, MinIO, API, worker가 필요하지 않다.
- RED는 source, service, database, lockfile을 변경하지 않는다.

## RED

```bash
just --justfile scripts/tasks/task-4b/mod.just red
printf 'task_4b_red_exit=%s\n' "$?"
```

유효한 RED는 test binary가 정상 compile된 뒤 다음 원인으로 nonzero 종료한다.

```text
RED: src/application/realtime/mod.rs is absent; task-4b must add outbox delivery, Redis tickets/PubSub, WebSocket transport, and static C1 composition
```

compile 오류, lockfile 변경, 다른 assertion 실패, 또는 source가 이미 존재해 card가
exit `2`로 거절한 결과는 유효한 RED가 아니다. 이 명령의 부작용은 Cargo
compile/test cache뿐이다. 2026-08-25 사용자 실행에서는 test target이 정상 compile된
뒤 예상한 realtime surface 부재 한 건으로 실패했고 `task_4b_red_exit=101`이
확인됐다.

## RED 이후 구현 경계

GREEN은 PostgreSQL DB clock과 lease-generation CAS를 사용하는 outbox worker,
Redis publish/retry/dead-letter, SHA-256 digest만 저장하는 one-time ticket, D13=A
expiry fence, local WebSocket registry와 heartbeat, denied-subscribe terminal cleanup,
두 단계 delta recovery, 실제 dev-fixture 기반 C1 흐름을 검증한다. Redis와
WebSocket은 delivery acceleration이며 PostgreSQL delta가 계속 correctness path다.

## Realtime dependency와 lock 검증

유효한 RED 뒤 다음 최소 dependency 경계를 추가했다.

- 기존 Axum `0.8.9`에 server WebSocket용 `ws` feature
- Redis Pub/Sub stream과 WebSocket test sink/stream용 `futures-util 0.3.34`
- 256-bit OS CSPRNG ticket 원문용 `getrandom 0.4.3`
- Redis에 저장할 ticket digest용 `sha2 0.10.9`
- 실제 loopback WebSocket client E2E에만 쓰는 dev dependency
  `tokio-tungstenite 0.29.0`

임의의 runtime registry, TLS backend, generic queue framework, 두 번째 database
transaction abstraction은 추가하지 않는다. `Cargo.lock`은 agent가 손으로 편집하지
않는다. 다음 card를 순서대로 실행한다.

```bash
just --justfile scripts/tasks/task-4b/mod.just locks
printf 'task_4b_locks_exit=%s\n' "$?"

just --justfile scripts/tasks/task-4b/mod.just dependency-check
printf 'task_4b_dependency_exit=%s\n' "$?"
```

`locks`는 task-1의 canonical Cargo/Nix lock 생성과 no-drift 검증을 재사용한다.
dependency card는 advisory, ban, license, source가 모두 통과해야 한다. 실패하면
finding 전체를 전달하고 예외나 무관한 version pin을 임의로 추가하지 않는다.
두 card 모두 exit `0`인 뒤에만 GREEN production surface를 구현한다.

2026-08-25 사용자 실행 evidence:

- `locks`: `task_4b_locks_exit=0`
- `Cargo.lock` SHA-256:
  `de00bfd644191e367eaa4979940c19655d87464d23551776e8f08ac336fa4a5e`
- 기존 `flake.lock` SHA-256:
  `31403f6a698d7386579ca297f53952fd8cb47616affa8ff49c9fc71517f05bd9`
- lock 재검증 뒤 두 lockfile 모두 byte drift 없음
- `dependency-check`: 중복 crate 경고만 남고
  `advisories ok, bans ok, licenses ok, sources ok`
- `task_4b_dependency_exit=0`

따라서 dependency gate는 통과했으며, 이 경고를 숨기기 위한 무관한 version pin이나
policy 예외는 추가하지 않는다.
