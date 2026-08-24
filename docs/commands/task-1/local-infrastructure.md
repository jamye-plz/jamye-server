# M0 local infrastructure card

## 목적

고정 project `jamye-server-test`에서 PostgreSQL, Redis, MinIO를 rootless Podman으로 명시적으로 관리한다. Compose는 local disposable integration test 전용이며 production SSOT가 아니다.

## macOS 1회 선행 조건

`podman machine`은 사용자 소유 host state다. 존재 여부를 먼저 확인하고, 필요한 경우에만 사용자가 직접 생성·시작한다.

```bash
nix develop path:.
podman machine list
podman machine init
podman machine start
```

기존 machine이 있으면 `init`을 실행하지 않는다. CPU, memory, disk 정책을 바꾸거나 machine을 삭제하는 명령은 이 card가 자동화하지 않는다.

Linux에는 machine 단계가 없으며 user session의 rootless Podman socket을 사용한다.

## 시작과 상태

먼저 environment card로 `.env.local`을 만든 뒤 실행한다.

```bash
nix develop path:. --command just task-1 infra-up
nix develop path:. --command just task-1 infra-status
```

`up`은 세 container를 시작하고 60초 안에 health를 확인한 뒤, MinIO 관리 계정으로 권한 없는 별도 local app 사용자를 만든다. bucket, policy, lifecycle은 만들지 않는다.

## 정지

```bash
nix develop path:. --command just task-1 infra-down
```

container와 project network만 내리고 세 named volume은 보존한다.

## Guarded reset

다음 명령은 세 local test volume의 bytes를 되돌릴 수 없게 삭제한다. command의 확인 문자열, top-level Compose project 이름, exact volume 이름, Compose/Podman ownership label이 모두 일치할 때만 진행한다.

```bash
JAMYE_CONFIRM_INFRA_RESET=jamye-server-test nix develop path:. --command just task-1 infra-reset
```

삭제 대상은 다음 세 개뿐이다.

```text
jamye-server-test-postgres-data
jamye-server-test-redis-data
jamye-server-test-minio-data
```

`.env.local`, 다른 project, Podman machine은 삭제하지 않는다.

## 부작용

- `podman machine init/start`는 macOS user-owned VM state를 만들거나 실행한다. repository script가 대신 수행하지 않는다.
- `infra up`은 세 container, project network, project-owned named volume을 만들고 local MinIO app identity를 추가한다.
- `infra down`은 container/network만 제거하고 volume bytes를 보존한다.
- guarded `infra reset`은 정확히 세 local test volume의 bytes를 복구 불가능하게 삭제한다.
- 어떤 명령도 production host/service/credential, homelab, bucket, policy, Git state를 변경하지 않는다.

## 성공 기준

- active Podman service가 rootless라고 보고한다.
- exactly `postgres`, `redis`, `minio`가 실행되고 healthy다.
- MinIO admin과 app identity가 분리돼 있고 M0 app identity에는 bucket policy가 없다.

## 복구

macOS에서 연결 실패 시 사용자가 `podman machine list`와 `podman machine start`를 확인한다. service health 실패 시 `just task-1 infra-status` 결과를 보존하고 `just task-1 infra-down`으로 안전하게 정지한다. data 초기화가 꼭 필요할 때만 guarded reset을 별도로 승인·실행한다.
