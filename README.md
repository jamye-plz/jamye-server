# jamye-server

`jamye-server`는 잼얘좀의 모바일 API, 실시간 이벤트 전달, 내구성 worker를 한 코드베이스에 담는 Rust/Axum modular monolith다. 기존 `jamye-plz`의 Python 코드를 줄 단위로 옮기지 않고, 검증된 제품 의미와 계약을 보존하면서 모바일·오프라인 복구에 맞는 경계를 새로 만든다.

현재 구현 단계는 M0다. 프로세스 기반, 설정 검증, health endpoint와 개발 환경을 먼저 고정하며 그룹·인증·메시지·미디어 기능은 각 후속 마일스톤이 소유한다. STT와 전사 기능은 이번 작업 및 C2 계약에서 제외되지만, 음성 파일 한 개를 가진 일반 채팅 메시지의 전송·동기화·재생은 유지한다.

## 저장소 역할

- `api`: REST, WebSocket, health endpoint를 제공하는 Axum binary
- `worker`: PostgreSQL의 durable intent를 처리하는 Tokio binary
- PostgreSQL: 사용자, 그룹, 메시지, 알림, conversation event, outbox의 authoritative state
- Redis: 커밋된 이벤트의 일시적 at-most-once fan-out
- MinIO: private object bytes 저장소. M0에서는 health만 확인하며 S3 자격증명·bucket·정책은 task-8이 소유한다.
- `contracts/`: 이후 server가 생성할 OpenAPI 3.1, realtime JSON Schema, fixture, manifest의 SSOT

모바일 앱의 SQLite outbox와 이 저장소의 PostgreSQL outbox는 서로 다른 책임을 가진다. 앱 outbox는 오프라인 전송 의도와 동일한 `client_msg_id` 재시도를 보존하고, 서버 outbox는 이미 PostgreSQL에 커밋된 event를 Redis로 발행한다. WebSocket 누락은 PostgreSQL 기반 REST delta sync로 복구한다.

## 개발 환경

개발 도구의 원본은 Nix flake다. mise와 rustup은 사용하지 않는다.

- `rust-toolchain.toml`: 정확한 Rust release/profile/components/targets의 유일한 원본
- `flake.nix` + `flake.lock`: Rust toolchain을 소비하고 Cargo 외 도구와 native dependency를 고정
- `Justfile`: 설치나 version pin을 하지 않는 작은 dispatcher
- `compose.yaml`: `jamye-server-test` 전용 rootless Podman local test harness

저장소가 아직 Git에 추가되지 않은 파일을 포함해도 같은 내용을 검증하도록 모든 로컬 flake 명령은 `path:.`를 사용한다. 시작점은 다음 한 명령이다.

```bash
nix develop path:.
```

devShell에 들어왔다고 서비스가 시작되지는 않는다. 다음 stable primitive만 제공한다.

```text
just cards
just task <task-id> <card>
just locks <create|verify>
just infra <status|up|down|reset>
```

정확한 명령, 목적, 부작용, 성공 기준, 복구 방법은 [task-1 command cards](docs/commands/task-1/README.md)에 있다. 이후 migration, contract 생성, API/worker 실행, feature test도 각 task card에 추가하고 README나 Justfile에 feature recipe를 넣지 않는다.

## M0 환경 계약

`.env.example`에는 key와 설명만 있으며 실제 값은 없다. M0 runtime이 소비하는 키는 다음 일곱 개뿐이다.

```text
JAMYE_ENVIRONMENT
JAMYE_LISTEN_ADDR
JAMYE_SHUTDOWN_GRACE_SECONDS
JAMYE_READINESS_TIMEOUT_MS
DATABASE_URL
REDIS_URL
JAMYE_MINIO_HEALTH_URL
```

task-1의 사용자 실행 card가 disposable `.env.local`을 만들며 이 파일은 Git 대상이 아니다. M0 server는 S3 credential, bucket, region, path-style 설정을 받지 않는다.

## 로컬 인프라

`compose.yaml`은 정확히 PostgreSQL, Redis, MinIO 세 서비스만 제공한다. loopback에만 bind하고 project-isolated named volume을 사용한다. MinIO 관리 계정과 별도 앱 계정은 분리하지만 M0 앱 계정에는 object policy를 부여하지 않고 bucket도 만들지 않는다. task-8에서 D11을 결정한 뒤 local policy와 production handoff를 구체화한다.

macOS의 `podman machine`은 저장소나 Nix가 소유하지 않는 사용자 관리 VM이다. 생성·시작·정지·삭제는 사용자가 명시적으로 실행한다. Linux에서는 같은 Compose project를 rootless Podman으로 직접 실행한다. devShell은 Compose provider를 Nix store 경로로 고정하므로 ambient Docker Compose나 Homebrew provider를 자동 선택하지 않는다.

`infra down`은 container와 network만 내리고 데이터를 보존한다. `infra reset`은 별도 확인 문자열, project 이름, 세 named-volume 이름과 ownership label을 검증한 뒤 그 local test bytes만 삭제한다. 에이전트는 이 명령을 실행하지 않는다.

## Health 의미론

- `GET /health/live`: process가 요청을 받을 수 있으면 항상 `200`
- `GET /health/ready`: PostgreSQL이 reachable일 때만 `200`
- Redis 또는 MinIO 장애: 응답에 `degraded`로 표시하지만 PostgreSQL이 정상이라면 readiness 자체는 실패시키지 않음
- PostgreSQL 장애: write/readiness 실패

로그는 JSON이다. caller의 `x-request-id`는 신뢰하지 않고 서버가 UUID를 발급해 response와 log에 같은 값을 전파한다. access/refresh token, OAuth code, realtime ticket, push token, 메시지 본문, object credential, presigned URL은 로그에 남기지 않는다. 종료 신호를 받으면 새 요청을 중단하고 설정된 grace period 안에서 in-flight 요청을 drain한다.

## 배포 경계

`compose.yaml`은 production 배포 정의가 아니다. 최종 flake는 `aarch64-darwin` 개발과 `x86_64-linux` production package/check를 내보내고, task-13에서 `nixosModules.default`를 완성한다. 현재 M0는 package/devShell/check 기반만 만든다.

별도 `homelab` 저장소가 다음을 소유한다.

- `jamye-server` flake revision pin
- SOPS secret과 `/run` environment file
- PostgreSQL, Redis, MinIO service와 volume
- host, domain, Caddy/Cloudflare ingress
- monitoring, alert, backup, restore drill

이 저장소의 최종 NixOS module은 package, listen address, environment file 경로, migration 실행 정책만 다룬다. 운영 DB, bucket, Redis, MinIO, homelab, 원격 저장소에 M0가 연결하거나 배포하지 않는다.

## 문서

- [로드맵](docs/roadmap.md)
- [아키텍처](docs/architecture.md)
- [의존성 baseline ADR](docs/adr/0001-dependency-baseline.md)
- [Nix/Just/Podman ADR](docs/adr/0002-nix-devshell-just-podman.md)
