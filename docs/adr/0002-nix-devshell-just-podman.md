# ADR 0002: Nix devShell, Just, rootless Podman responsibility split

- 상태: Accepted
- 날짜: 2026-08-24

## Context

개발자는 macOS Apple Silicon에서 작업하고 production target은 x86_64 Linux/NixOS다. Rust version, native tool, task command, local infrastructure, production deployment의 소유자를 겹치게 두면 다음 문제가 생긴다.

- Nix와 mise/rustup이 같은 tool version을 서로 다르게 고정
- `podman compose`가 ambient Docker Compose 또는 다른 provider를 선택
- local Compose가 production 배포 정의로 오해됨
- agent가 podman machine이나 service lifecycle을 암묵적으로 변경
- README/Justfile이 feature command를 계속 흡수해 변경 충돌 지점이 됨

## Considered options

### A. Nix devShell + mise + Docker Compose

Nix는 native package, mise는 language/task version, Docker는 infra를 맡는다. 익숙한 예제가 많지만 tool authority가 중복되고 사용자가 선택한 Podman과 맞지 않는다.

### B. Nix devShell + raw flake app/shell script, task runner 없음

authority는 단순하지만 discoverability가 낮다. 긴 command를 사용자가 매번 복사해야 하고 task owner별 evidence 위치가 불명확해진다.

### C. Nix devShell + small Just dispatcher + rootless Podman Compose

Nix가 모든 executable/version을 제공하고 Just는 설치 기능이 없는 catalog/dispatcher만 맡는다. Compose는 local disposable service에만 사용하고 production은 native NixOS/systemd로 유지한다.

## Decision

Option C를 채택한다.

### Rust/Nix boundary

`rust-toolchain.toml`만 exact Rust release/profile/components/targets를 선언한다. flake는 Fenix로 그 파일을 읽고 manifest integrity hash만 제공한다. devShell과 Crane package는 같은 toolchain derivation을 사용한다. `flake.nix`/`flake.lock`은 nixpkgs, Fenix, Crane, native dependency와 non-Rust CLI를 소유한다.

mise, rustup, `.tool-versions`, 별도 Rust version literal을 추가하지 않는다.

### Just boundary

Justfile은 다음 stable primitive만 가진다.

- `nix develop path:.` 진입 안내
- card 목록
- `scripts/tasks/<task-id>/<card>.sh`의 guarded dispatch
- task-1 lock wrapper
- fixed local infra lifecycle wrapper

정확한 feature command와 목적/부작용/성공 기준/복구는 `docs/commands/<task-id>/`와 `scripts/tasks/<task-id>/`에 둔다. 이후 feature task는 README/Justfile에 recipe를 추가하지 않는다. Just는 tool을 설치하거나 version을 고정하지 않는다.

### Podman boundary

`compose.yaml`은 top-level project `jamye-server-test`와 정확히 PostgreSQL, Redis, MinIO 세 local service만 가진다. loopback port, fixed non-latest image tag, healthcheck, project-owned named volume을 사용한다.

devShell은 Podman client와 `podman-compose`를 제공하며 `PODMAN_COMPOSE_PROVIDER`를 Nix store executable에 고정한다. provider/version card는 service를 시작하지 않고 ambient fallback이 없는지 먼저 확인한다.

macOS `podman machine`은 user-owned prerequisite다. repository/Just/agent는 VM을 만들거나 시작·정지·reset·삭제하지 않는다. Linux는 VM layer 없이 같은 rootless project를 사용한다.

service lifecycle도 사용자가 명시적으로 card를 실행해야만 바뀐다. `down`은 bytes를 보존한다. `reset`은 별도 confirmation, exact project, exact volume 이름과 ownership label을 검증해 local test volume 세 개만 삭제한다.

M0 MinIO bootstrap은 disposable admin과 별도 app identity만 준비한다. app은 권한이 없고 bucket/policy/lifecycle은 만들지 않는다. task-8에서 사용자가 D11을 선택하기 전까지 production 또는 local object policy를 선점하지 않는다.

### Production boundary

Compose는 production SSOT가 아니다. production은 flake의 native `api`/`worker` package와 task-13 NixOS module을 homelab이 pin해 systemd로 실행한다.

server repository의 module은 package, listen address, environment file, migration policy만 소유한다. homelab이 PostgreSQL/Redis/MinIO, host/domain/volume, SOPS secret, ingress, monitoring, backup/restore를 소유한다. 이 저장소의 env/flake/module에 production value를 넣지 않는다.

## Supported systems and validation

- `aarch64-darwin`: development devShell, package, checks
- `x86_64-linux`: production package, checks, devShell evaluation

모든 pre-SCM flake command는 Git index 밖의 파일도 포함하도록 `path:.`를 사용한다. macOS에 Linux builder가 없으면 PF2 hard blocker로 기록하고 Linux package 성공을 주장하지 않는다.

사용자는 task-1 cards로 다음 evidence를 반환한다.

1. exact Nix Podman/Compose provider path와 version
2. Cargo/Nix lock checksum과 no-drift
3. local format/lint/test/check
4. rootless three-service health
5. aarch64-darwin current-system flake check
6. x86_64-linux api/worker package build 또는 explicit builder blocker

## Consequences

- 장점: version authority와 lifecycle authority가 한눈에 보인다.
- 장점: agent가 중요한 host/service state를 암묵적으로 바꾸지 않는다.
- 장점: local service harness와 production deployment가 섞이지 않는다.
- 비용: 사용자가 command card를 직접 실행하고 evidence를 돌려줘야 gate가 닫힌다.
- 비용: macOS의 Linux production build에는 별도 builder가 필요하다.
- 위험: nixpkgs의 Podman Darwin availability 또는 external provider compatibility가 lock revision에서 깨질 수 있다. provider card 실패 시 ambient 도구로 우회하지 않고 flake를 수정한다.
- 위험: fixed Compose image의 security/availability는 local compatibility 목적에만 유효하다. production 선택으로 전이하지 않는다.
