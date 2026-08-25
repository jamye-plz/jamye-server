# ADR 0004: Redis-only one-time realtime tickets and durable PostgreSQL delivery

- 상태: Accepted; task-4b GREEN/recovery evidence complete
- 날짜: 2026-08-25
- 범위: R1 realtime ticket, WebSocket admission, outbox delivery acceleration

## Context

WebSocket은 일반 Bearer를 query string에 오래 노출하지 않고도 짧은 연결 권한을
받아야 한다. 동시에 Redis와 WebSocket은 메시지의 authoritative source가 아니다.
메시지, conversation event, outbox intent와 복구 cursor는 PostgreSQL에 남아야 하며,
Redis 유실이나 재시작이 committed message를 잃게 해서는 안 된다.

선택된 D13=A에서는 logout이 이미 발급된 짧은 access token을 즉시 폐기하지 않는다.
따라서 ticket과 established socket의 최종 권한도 그 token의 `exp`를 넘지 않는
명시적 deadline으로 묶어야 한다.

## Decision

### Ticket material

- R1은 검증된 Bearer와 정확한 `X-Jamye-Contract-Version`을 요구한다. current `1`과
  previous `0`만 받으며 unsupported version은 credential을 만들기 전에
  `426 contract_upgrade_required`를 반환한다.
- 원문 ticket은 OS CSPRNG의 256 bit를 lowercase hexadecimal 64자로 인코딩하고 응답에
  한 번만 반환한다.
- Redis key에는 `SHA-256(raw ticket)`만 사용한다. 원문, Bearer, signing secret은
  저장하거나 log field에 넣지 않는다.
- Redis value는 `user_id`, 검증된 signed access-token `sid`, negotiated
  `contract_version`, `access_token_expires_at`만 담는다.
- TTL은 양수인 `min(issue time + 30 seconds, access-token exp) - issue time`이다.
  `SET NX PX`로 digest collision을 거절하고 `GETDEL`로 정확히 한 consumer만 record를
  얻는다.
- malformed, expired, reused, Redis restart로 사라진 ticket은 모두 upgrade 뒤
  `4401 realtime_auth_failed`로 동일하게 닫힌다. Redis 자체가 unavailable이면
  ticket POST와 pre-upgrade handshake는 `503 realtime_unavailable`을 반환한다.

### Established socket

Socket은 consumed record의 `access_token_expires_at`을 보유한다. deadline에 도달하면
해당 socket의 user mapping과 모든 conversation subscription을 먼저 원자적으로
제거한 뒤 `4401 realtime_auth_expired`로 닫는다. 유효 membership을 확인하고 local
registry에 등록한 뒤에만 `subscribed` ack를 보낸다. 거부된 subscribe는 기존의 다른
subscription까지 먼저 제거하고 data/error frame 없이 `4001 membership_required`로
닫는다.

Application heartbeat는 client ping 25초와 추가 pong deadline 10초를 사용한다.
protocol error는 `4400`, 내부 dependency error는 `1011`이다. local connection registry는
구체 transport 구현이며 application port나 runtime plugin registry가 아니다. 각 socket의
outbound queue는 bounded이며, 가득 찬 queue에 event를 무한 적재하지 않는다. 이때 누락된
low-latency hint는 reconnect/delta로 복구한다.

### Durable delivery

Message REST transaction이 message, conversation event, outbox intent를 한 번에 commit한
뒤 worker가 outbox를 claim한다.

- claim 대상과 lease expiry는 PostgreSQL `clock_timestamp()`가 결정한다.
- concurrent worker는 `FOR UPDATE SKIP LOCKED`로 같은 row를 동시에 claim하지 않는다.
- claim 때마다 `claim_generation`을 증가시킨다. publish marker와 retry/dead-letter
  mutation은 `id`, `claimed` status, owner, captured generation, 아직 살아 있는 lease를
  모두 compare-and-set한다.
- 같은 owner string을 재사용해도 stale generation은 newer state를 바꾸지 못한다.
- Redis publish timeout과 safety margin의 합은 lease보다 반드시 짧다.
- 한 batch에서 claim한 row의 publish/CAS future는 같은 lease budget 안에서 함께 시작한다.
- failure는 safe code, attempt count와 next-attempt DB time을 저장한다. max attempt 또는
  deadline을 넘으면 row를 지우지 않고 `dead_letter`와 terminal timestamp를 남긴다.
- Redis publish 뒤 PostgreSQL marker 전에 worker가 죽으면 같은 stable `event_id`가
  중복 publish될 수 있다. exactly-once transport를 주장하지 않으며 client는 event ID로
  idempotently apply하고 two-phase delta drain으로 cursor를 단조 증가시킨다.

## Failure and recovery boundary

Redis가 중지되어도 authenticated message REST write와 PostgreSQL delta는 계속
correctness path다. outbox row는 persisted retry state로 남고 같은 worker/router가 Redis
재시작 뒤 다시 연결해 publish한다. local test Redis는 persistence를 끈 disposable
instance이므로 restart 전 ticket은 의도적으로 사라지며 새 ticket이 필요하다.

복구 순서는 refresh 또는 재인증, 새 ticket, delta phase #1 완전 drain, subscribe ack,
delta phase #2 완전 drain이다. WebSocket에서 본 known event도 phase #2 cursor를 건너뛰게
하지 않으며, unknown WebSocket type은 cursor를 전진시키지 않고 S1을 요구한다.

## Operational boundary

- `compose.yaml`의 Redis는 loopback disposable integration fixture다. production Redis
  topology, persistence, failover, capacity를 결정하지 않는다.
- 실제 local Redis stop/start는 task-4b recovery card를 사용자가 명시적으로 실행할 때만
  수행한다. card는 Redis container 하나만 다루고 volume reset을 호출하지 않는다.
- PostgreSQL migration, production deployment, homelab service lifecycle은 이 ADR의
  실행 권한에 포함되지 않는다.

## Consequences

- 장점: raw Bearer나 장기 credential을 WebSocket query에 사용하지 않는다.
- 장점: Redis 유실과 realtime 중단이 committed message/delta correctness를 훼손하지
  않는다.
- 장점: generation CAS가 same-owner ABA와 늦은 worker mutation을 차단한다.
- 비용: Redis restart 뒤 모든 미소비 ticket을 다시 발급해야 한다.
- 비용: at-most-once Pub/Sub 위에서는 duplicate/missed delivery를 전제로 client delta
  recovery가 항상 필요하다.

## References

- [Frozen realtime protocol](../../contracts/realtime/protocol.json)
- [Task-4b command cards](../commands/task-4b/realtime.md)
- [Forward-only PostgreSQL migrations](0003-forward-only-sqlx-migrations.md)
