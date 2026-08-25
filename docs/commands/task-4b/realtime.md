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

현재 Cargo graph에는 Axum WebSocket feature와 ticket용 직접 SHA-256/CSPRNG
dependency가 없다. 따라서 유효한 RED evidence를 받은 뒤 task-4b 구현에 필요한
최소 manifest delta를 먼저 검토하고, 사용자가 lock/no-drift 및 dependency card를
실행한 evidence를 받은 다음 GREEN 구현을 진행한다. RED 전에는 manifest나
lockfile을 바꾸지 않는다.
