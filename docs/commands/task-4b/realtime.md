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

## GREEN

GREEN은 다음 경계를 한 target에서 검증한다.

- PostgreSQL DB clock, `SKIP LOCKED`, lease generation과 live-lease CAS
- persisted retry/dead-letter와 stable `event_id`
- Redis digest-only ticket, positive bounded TTL, concurrent `GETDEL`, Pub/Sub payload
- ordered subscribe/unsubscribe, application ping/pong, heartbeat, denied cleanup,
  access-expiry cleanup
- dev seed → Bearer → R1 → WS subscribe → REST message → outbox worker → Redis → WS →
  S1 terminal delta의 실제 loopback C1
- 두 단계 delta observer의 join-gap, duplicate/out-of-order, non-progress와 unknown marker
- default/all-feature clean-architecture 경계

PostgreSQL과 Redis가 healthy인 상태에서 실행한다.

```bash
just --justfile scripts/tasks/task-4b/mod.just green
printf 'task_4b_green_exit=%s\n' "$?"
```

성공 기준은 realtime target의 non-ignored test가 전부 통과하고 architecture 4개가
통과하며 최종 exit가 `0`인 것이다. compile warning, test skip, Redis/PostgreSQL 연결
실패는 성공 evidence가 아니다. 이 card는 service lifecycle을 변경하지 않는다.

2026-08-25 첫 사용자 실행은 `tungstenite 0.29`의 `Utf8Bytes`가 여러 `AsRef`
구현을 제공해 WebSocket close reason assertion 5곳에서 E0283으로 compile 실패했다.
문자열 비교 의도를 `as_str()`로 명시한 뒤 재실행에서는 realtime 21/21과
architecture 4/4가 모두 통과했고 `task_4b_green_exit=0`을 확인했다. recovery target
한 개는 이 card에서 의도적으로 분리됐다.

## 실제 Redis stop/start recovery

GREEN 통과 뒤 다음 card를 별도로 실행한다.

```bash
just --justfile scripts/tasks/task-4b/mod.just redis-recovery
printf 'task_4b_redis_recovery_exit=%s\n' "$?"
```

이 recipe가 Bash를 사용하는 이유는 `trap` cleanup, Rust test와의 marker coordination,
bounded health wait, 중단 시 Redis 복구가 필요한 상태ful safety boundary이기 때문이다.
일반 Cargo 순서와 dependency 검사는 계속 Just module에 직접 둔다.

Recovery card는 exact `jamye-server-test` Compose project의 Redis container 하나만
중지·재시작한다. PostgreSQL과 named-volume bytes는 보존하며 `infra-reset`을 호출하지
않는다. 같은 in-process Router/worker가 다음을 증명해야 한다.

1. Redis outage 중 outbox가 `pending` retry와 safe error code를 유지한다.
2. ticket POST와 pre-upgrade WebSocket handshake는 safe `503 realtime_unavailable`이다.
3. 같은 idempotency key REST retry와 PostgreSQL delta는 계속 성공한다.
4. restart 전 ticket은 `4401 realtime_auth_failed`로 닫힌다.
5. 새 ticket/subscription 뒤 같은 worker가 durable outbox를 publish하고 row를
   `published`로 전환한다.

중간에 중단되면 trap이 Redis를 다시 시작하고 최대 60초 동안 health를 기다린다.
그 복구도 실패하면 다음을 먼저 실행해 exact project status를 확인한다.

```bash
just task-1 infra-status
```

volume 삭제는 기본 복구가 아니다. 전체 disposable reset은 별도의 파괴적
`just task-1 infra-reset` 승인을 받은 경우에만 사용한다.

2026-08-25 사용자 실행에서는 guarded Redis container만 실제로 중지·재시작하는
ignored recovery target 1/1이 통과했다. 같은 Router와 worker가 재시작 뒤 복구됐고
PostgreSQL outbox bytes가 보존됐으며 `task_4b_redis_recovery_exit=0`을 확인했다.

설계 근거는 [ADR 0004](../../adr/0004-realtime-ticket-storage.md)에 기록한다.
