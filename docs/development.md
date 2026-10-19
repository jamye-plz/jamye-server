# 로컬 개발

`just`를 실행하면 모든 공개 명령을 볼 수 있다. repository가 고정한 Nix devShell이
이미 활성화돼 있어야 하며, 그렇지 않으면 각 명령은 즉시 실패한다.

## devShell 진입과 세션 재사용

프로젝트 루트에서 다음 명령으로 devShell을 한 번 열고 작업이 끝날 때까지 재사용한다.
서비스나 애플리케이션 빌드를 시작하는 명령은 아니다. GC 이후에는 고정 도구를 다시
다운로드하거나 개발 환경을 구성할 수 있다.

```bash
nix develop . --no-write-lock-file --command bash --noprofile --norc
```

- 에이전트는 `jamye-server`와 `jamye-app`의 터미널 세션을 각각 유지한다. 후속 명령은
  해당 프로젝트의 같은 세션에서 실행하며, 명령마다 `nix develop`을 중첩하지 않는다.
- 사용자가 별도로 연 터미널의 환경을 상속한다고 가정하지 않는다. 세션 ID와 작업 디렉터리는
  실행을 담당하는 에이전트가 관리하고, 첫 진입 때 `IN_NIX_SHELL`과 도구 버전을 확인한다.
  비밀이 포함될 수 있는 전체 환경 변수는 출력하지 않는다.
- 하위 에이전트는 새 devShell을 개별 생성하지 않고 필요한 명령을 coordinator에 요청한다.
  한 세션의 입력은 한 실행 주체가 순서대로 관리한다.
- 세션이 종료됐거나 `flake.lock`, devShell 또는 toolchain 설정이 바뀌었을 때만 해당 세션을
  다시 연다. 일반 소스 수정이나 검사 재시도 때문에 환경을 다시 만들지 않는다.

Git 저장소에서는 `path:.` 대신 `.`을 사용한다. flake 입력은 Git 추적 파일의 working tree로
제한되므로 ignored `target/` 같은 산출물이 포함되지 않는다. 추적 중인 파일의 미커밋 수정도
사용하지만 새 Nix 입력 파일은 Git 추적 여부를 확인해야 한다. 이를 우회하려고 전체 `path:.`
입력으로 돌아가거나 관련 없는 파일을 자동 stage하지 않는다. devShell 안의 로컬 명령은
원래 작업 디렉터리에서 실행하므로 새 소스 파일도 읽을 수 있다.

환경 진입은 테스트·빌드·로컬 서비스·운영 데이터 변경의 포괄 승인이 아니다. 필요한 작업만
실행하며 `nix flake check`를 매번 진입 전 검사로 붙이지 않는다. 로컬 `target/`은 재생성 가능한
Cargo 산출물이며 원격 NixOS 서비스와 별개지만, 정리 후 테스트·계약 생성은 재컴파일이 필요하다.

## 로컬 서비스

1. `just env-create`를 한 번 실행해 mode `0600`의 `.env.local`을 만든다.
2. `just infra-up`으로 Compose의 PostgreSQL·Redis와 flake-pinned host MinIO를
   시작하고 health 상태까지 기다린다. 최초 MinIO 실현은 소스 빌드가 필요할 수 있다.
3. `just infra-status`로 상태를 확인한다.
4. `just minio-policy`로 분리된 app identity에 테스트용 최소 권한을 연결한다.
5. `just infra-down`으로 data를 보존한 채 서비스를 정지한다.

기본 engine은 rootless Podman이며 devShell이 Compose provider를 Nix store에 고정한다.
macOS의 Podman machine은 사용자 소유이고 repository 명령은 이를 생성·시작·정지·삭제하지
않는다. CI처럼 Docker를 사용할 때는 `JAMYE_TEST_CONTAINER_ENGINE=docker`를 설정한다.
PostgreSQL과 Redis는 loopback에 bind하고 project로 격리된 named volume을 사용한다.

MinIO는 `.env.local`의 관리자 자격증명을 process 환경으로 받고 loopback
(`127.0.0.1:9000`/`9001`과 함께 열리는 IPv6 loopback `[::1]`)에만 bind한다.
data, PID, log는 Git에서 제외된
`.local/infra/minio/`에 보관한다. 반복 `infra-up`은 같은 process를 재사용한다.
`infra-down`은 TERM으로 최대 10초간 종료를 기다리며 강제 종료하지 않는다. PID가 다른
process를 가리키거나 포트가 외부 process에 점유돼 있으면 exit 2로 중단하며 해당
process에는 신호를 보내지 않는다.
리스너 소유 확인에는 macOS의 `lsof` 또는 Linux의 `ss`가 필요하다.

이전 Compose MinIO를 사용했다면 첫 `infra-up`이 project와 `minio` service label이
일치하는 기존 container만 제거해 포트를 비운다. 기존 `jamye-server-test-minio-data`
volume은 유지되지만 새 host data 경로로 자동 이전되지는 않는다. 기존 데이터가 필요하면
reset 전에 별도로 보관·이전한다.

`JAMYE_CONFIRM_INFRA_RESET=jamye-server-test just infra-reset`은 project와 volume
ownership label 및 고정 MinIO 경로를 검증하고 MinIO 종료에 성공한 뒤 PostgreSQL·Redis
volume, 존재하는 legacy MinIO volume, host MinIO data/PID/log를 삭제한다. 확인값이
없으면 삭제하지 않으며 `.env.local`은 항상 보존한다.

## Process와 health

- `just run-api`는 `.env.local`을 읽어 API를 실행한다.
- `just run-worker`는 `.env.local`을 읽어 background worker를 실행한다.
- `just health-check`는 실행 중인 API의 `/health/live`와 PostgreSQL-backed readiness를
  확인한다.

구현 완료와 recovery gate는 [검증 절차](validation.md)를 참고한다.
