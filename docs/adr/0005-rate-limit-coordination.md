# ADR 0005: Redis-coordinated fixed-window rate limiting

- 상태: Accepted
- 날짜: 2026-08-25
- 범위: 인증, 초대, presign 등 mutation 전 shared rate-limit 경계

## Context

인증 endpoint는 여러 API process가 동시에 처리할 수 있다. Process-local counter는 node가
늘어날 때 제한을 우회할 수 있고, read 후 write 방식은 동시 요청을 잃는다. 반대로
rate-limit 상태는 refresh session 같은 authoritative 보안 상태가 아니므로 PostgreSQL
transaction 안에 넣거나 Redis 복구를 session 복구와 결합해서도 안 된다.

D6=A는 제품 수치를 새 사용자 gate로 열지 않고 보수적 기본값을 runtime config로 조정할
수 있게 한다. Task-5는 공통 port와 adapter를 한 번 소유하고 task-6의 invite와 task-8의
presign이 같은 경계를 재사용한다.

## Decision

### 원자적 counter

- `RateLimiter` port는 endpoint, actor/IP subject, 양수 limit, 0이 아닌 window를 받아
  `Allowed` 또는 정확한 `retry_after`가 있는 `Denied`만 반환한다. Application은 raw Redis
  command나 key를 알지 못한다.
- Redis adapter는 Lua 한 번으로 `INCR`하고 첫 increment일 때만 `PEXPIRE`를 설정한 뒤
  `PTTL`을 함께 반환한다. 별도 read/write round trip은 허용하지 않는다.
- key namespace는 `jamye:rate-limit:v1:<endpoint>:<sha256(subject)>`다. Endpoint는
  lowercase ASCII, digit, underscore만 허용하고 actor/IP/refresh credential 원문은 key나
  structured log에 넣지 않는다.
- Endpoint와 subject는 서로 격리된다. 여러 adapter instance가 같은 Redis를 사용해도 한
  counter를 공유하며 concurrency 아래 lost update가 없어야 한다.
- Task-5 기본값은 minute window에서 authorize 10, exchange 20, refresh 30, logout 30이다.
  Behavior test는 이 운영 기본값에 기대지 않고 작은 명시적 fixture limit를 사용한다.

### 실패와 HTTP 의미

- 보호된 use case는 rate-limit check를 OAuth attempt 생성, provider network I/O,
  PostgreSQL transaction과 session/profile mutation보다 먼저 수행한다.
- Limit 초과는 `429 rate_limit_exceeded`와 정수 `Retry-After` header를 반환한다.
- Redis connection/script 실패는 fail-closed `503 rate_limit_unavailable`로 축약한다.
  Driver error, Redis URL, subject나 credential은 응답과 log에 노출하지 않는다.
- Redis restart는 counter를 보존하거나 초기화할 수 있다. 어느 쪽도 PostgreSQL의 hashed
  refresh-session authority를 만들거나 revoke하거나 변경하지 못한다.
- Message write와 delta처럼 task-5 rate-limit 보호 대상이 아닌 correctness path는 Redis
  limiter 장애 때문에 새 제한을 받지 않는다.

## Validation and recovery

Task-5 integration test는 두 adapter instance의 16개 동시 increment에서 fixture limit 8을
정확히 공유하고, 8개 허용·8개 거절, endpoint/subject 격리, TTL reset과 raw-subject 비노출을
검증한다. 주입형 outage test는 `503`이 모든 mutation 전에 반환됨을 검증한다.

실제 local recovery card는 guarded loopback Redis container 하나만 중지·재시작한다. 같은
`RedisRateLimiter` instance가 중지 중 실패하고 재시작 뒤 다시 연결되는 동안 PostgreSQL에
저장된 refresh token digest bytes가 바뀌지 않는지 확인한다. Volume reset이나 production
service 조작은 이 card의 권한이 아니다.

## Consequences

- 장점: node 수와 관계없이 한 atomic counter가 제한을 강제한다.
- 장점: Redis key와 log가 actor/IP/refresh credential 원문을 보존하지 않는다.
- 장점: limiter 장애가 session authority를 손상시키지 않고 명시적 503으로 닫힌다.
- 비용: fixed window 경계에서 burst가 몰릴 수 있으며 sliding-window 공정성을 제공하지
  않는다.
- 비용: Redis 유실 뒤 counter continuity를 보장하지 않으므로 이 mechanism은 abuse 완화이지
  durable audit ledger가 아니다.

## References

- [Task-5 authentication command cards](../commands/task-5/auth.md)
- [Redis `EVAL`](https://redis.io/docs/latest/commands/eval/)
- [Redis `INCR`](https://redis.io/docs/latest/commands/incr/)
- [Forward-only PostgreSQL migrations](0003-forward-only-sqlx-migrations.md)
