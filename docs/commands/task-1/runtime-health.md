# M0 runtime and health card

## 목적

local infra가 healthy인 상태에서 API/worker를 실행하고 live/readiness 의미론을 확인한다.

## 명령

먼저 infrastructure card의 `infra up`을 완료한다. 별도 terminal A에서 API를 실행한다.

```bash
nix develop path:.
just task task-1 api-run
```

별도 terminal B에서 worker를 실행한다.

```bash
nix develop path:.
just task task-1 worker-run
```

별도 terminal C에서 health를 확인한다.

```bash
nix develop path:. --command just task task-1 health-check
```

## 부작용

API가 `.env.local`의 loopback listen address를 점유하고 DB/Redis/MinIO health connection을 연다. worker M0 skeleton도 process를 시작한다. source, lock, schema, bucket, Git state를 변경하지 않는다.

## 성공 기준

- `/health/live`는 process가 떠 있을 때 `200` JSON을 반환한다.
- PostgreSQL이 reachable이면 `/health/ready`가 `200`이다.
- platform integration target의 독립 probe double은 Redis/MinIO 실패를 `degraded` + HTTP `200`으로, PostgreSQL 실패를 HTTP `503`으로 검증한다.
- integration target은 in-flight request drain과 server-issued request ID를 검증하고, Unix runtime은 같은 drain 경계에 SIGTERM/SIGINT handler를 연결한다.

실제 Redis/MinIO/PostgreSQL 중단·재시작 시나리오는 이 card가 임의로 서비스를 변경해 증명하지 않는다. 각 failure/recovery milestone의 owner가 task-owned command card를 추가해 task-1의 lifecycle primitive를 호출한다.

## 복구

port 충돌이면 `JAMYE_LISTEN_ADDR`의 local 값을 사용자가 바꾸고 다시 실행한다. API/worker는 각 terminal에서 종료 신호를 보내 정상 종료한다. infra는 별도 `infra down` card로 내린다.
