# jamye-server

`jamye-server`는 잼얘좀의 모바일 API, 실시간 이벤트 전달, 내구성 worker를 한 코드베이스에 담는 Rust/Axum modular monolith다. 기존 `jamye-plz`의 Python 코드를 줄 단위로 옮기지 않고, 검증된 제품 의미와 계약을 보존하면서 모바일·오프라인 복구에 맞는 경계를 새로 만든다.

기존 FastAPI 백엔드의 승인된 기능 이관과 모바일 C2 서버 계약은 완료됐다. 현재 저장소는 `api`, `worker`, `admin` binary와 NixOS module을 제공하며, production 배포는 별도 `homelab` 저장소가 소유한다. STT와 전사 기능은 승인된 non-goal이지만, 음성 파일 한 개를 가진 일반 채팅 메시지의 전송·동기화·재생은 유지한다.

## 저장소 역할

- `api`: REST, WebSocket, health endpoint를 제공하는 Axum binary
- `worker`: PostgreSQL의 durable intent를 처리하는 Tokio binary
- `admin`: SSH에서 moderation 작업을 수행하는 operator CLI
- PostgreSQL: 사용자, 그룹, 메시지, 알림, conversation event, outbox의 authoritative state
- Redis: 커밋된 이벤트의 일시적 at-most-once fan-out
- MinIO: media와 account cleanup object를 보관하는 private object store. NixOS module이 S3 credential 분리와 bucket policy provisioning을 구성한다.
- `contracts/`: 이후 server가 생성할 OpenAPI 3.1, realtime JSON Schema, fixture, manifest의 SSOT

모바일 앱의 SQLite outbox와 이 저장소의 PostgreSQL outbox는 서로 다른 책임을 가진다. 앱 outbox는 오프라인 전송 의도와 동일한 `client_msg_id` 재시도를 보존하고, 서버 outbox는 이미 PostgreSQL에 커밋된 event를 Redis로 발행한다. WebSocket 누락은 PostgreSQL 기반 REST delta sync로 복구한다.

## 개발 환경

개발 도구의 원본은 Nix flake다. mise와 rustup은 사용하지 않는다.

- `rust-toolchain.toml`: 정확한 Rust release/profile/components/targets의 유일한 원본
- `flake.nix` + `flake.lock`: Rust toolchain을 소비하고 Cargo 외 도구와 native dependency를 고정
- `Justfile`: 설치나 version pin을 하지 않는 단일 공개 command catalog
- `compose.yaml`: `jamye-server-test` 전용 rootless Podman local test harness

flake 명령은 Git-tracked working tree를 source로 사용해 `target/` 같은 local output을 Nix store에 복사하지 않는다. 새 flake 입력 파일은 검증 전에 Git index에 추가해야 한다. 개발 도구는 pinned Nix devShell에서 제공한다.

```bash
nix develop .
```

devShell에 들어왔다고 서비스가 시작되지는 않는다. 지원하는 명령은 루트에서 바로 확인하고 실행한다.

에이전트는 프로젝트별 devShell을 한 번 열어 후속 명령에 재사용한다. 명령마다 환경을 다시
열거나 `path:.`로 로컬 산출물까지 flake 입력에 포함하지 않는다. 자세한 운영 규칙은
[로컬 개발](docs/development.md#devshell-진입과-세션-재사용)을 따른다.

```text
just
just check
just infra-status
just test-recovery
```

`just check`는 format, strict Clippy, default/all-feature tests와 contract drift를 한 번에 실행하는 구현 완료 게이트다. 서비스 stop/start recovery와 flake 검증은 각각 `just test-recovery`, `just flake-check`로 분리한다. 자세한 의미는 [검증 절차](docs/validation.md)와 [로컬 개발](docs/development.md)에 있다.

## 로컬 테스트 환경

`.env.example`은 공개 설정 형식을 설명하고 실제 값은 포함하지 않는다. `just env-create`는 로컬 통합 테스트용 `.env.local`을 mode `0600`으로 한 번만 생성하며 기존 파일을 덮어쓰지 않는다. `.env.local`은 Git 대상이 아니다.

local-dev 키의 목적·기본값·소유 모듈은 `.env.example`에, production 키와 검증은 `src/config/**`, 개발 fixture 가드는 `src/dev_fixtures`에 있다. production secret이나 운영 값을 `.env.example`에 넣지 않는다.

## 로컬 인프라

로컬 PostgreSQL과 Redis는 Compose에서, MinIO는 flake가 고정한 host process로 실행한다.
시작·종료·reset과 기존 MinIO 데이터 전환은 [로컬 개발](docs/development.md#로컬-서비스)을,
CI 게이트는 [검증 절차](docs/validation.md)를 참고한다.

## Health 의미론

- `GET /health/live`: process가 요청을 받을 수 있으면 항상 `200`
- `GET /health/ready`: PostgreSQL이 reachable일 때만 `200`
- Redis 또는 MinIO 장애: 응답에 `degraded`로 표시하지만 PostgreSQL이 정상이라면 readiness 자체는 실패시키지 않음
- PostgreSQL 장애: write/readiness 실패

로그는 JSON이다. caller의 `x-request-id`는 신뢰하지 않고 서버가 UUID를 발급해 response와 log에 같은 값을 전파한다. access/refresh token, OAuth code, realtime ticket, push token, 메시지 본문, object credential, presigned URL은 로그에 남기지 않는다. 종료 신호를 받으면 새 요청을 중단하고 설정된 grace period 안에서 in-flight 요청을 drain한다.

## 배포 경계

`compose.yaml`은 production 배포 정의가 아니다. flake는 `aarch64-darwin`, `aarch64-linux`, `x86_64-linux`의 `api`/`worker`/`admin` package, checks와 `nixosModules.default`를 내보낸다. NixOS module의 systemd 서비스와 operator `jamye-server-admin` wrapper는 flake check로 평가한다. 실제 host, secret, 서비스 lifecycle, ingress와 배포 결과는 별도 `homelab` 저장소의 책임이다.

별도 `homelab` 저장소가 다음을 소유한다.

- `jamye-server` flake revision pin
- SOPS secret과 `/run` environment file
- PostgreSQL, Redis, MinIO service와 volume
- host, domain, Caddy/Cloudflare ingress
- monitoring, alert, backup, restore drill

이 저장소의 NixOS module은 PostgreSQL, Redis, MinIO, MinIO identity 초기화, migration, api/worker systemd unit과 `jamye-server-admin` wrapper를 정의한다. host, secret 값, ingress, volume 정책과 배포는 homelab이 소유한다.

## 문서

- [로드맵](docs/roadmap.md)
- [검증 절차](docs/validation.md)
- [로컬 개발](docs/development.md)
- [아키텍처](docs/architecture.md)
- [의존성 baseline ADR](docs/adr/0001-dependency-baseline.md)
- [Nix/Just/Podman ADR](docs/adr/0002-nix-devshell-just-podman.md)
- [루트 Just 검증 ADR](docs/adr/0007-root-just-validation.md)
