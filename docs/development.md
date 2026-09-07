# 로컬 개발

`just`를 실행하면 모든 공개 명령을 볼 수 있다. repository가 고정한 Nix devShell이
이미 활성화돼 있어야 하며, 그렇지 않으면 각 명령은 즉시 실패한다.

## 로컬 서비스

1. `just env-create`를 한 번 실행해 mode `0600`의 `.env.local`을 만든다.
2. `just infra-up`으로 repository 소유의 PostgreSQL, Redis, MinIO test service를
   시작하고 health 상태까지 기다린다.
3. `just infra-status`로 상태를 확인한다.
4. `just infra-down`으로 data를 보존한 채 container를 정지한다.

`just infra-reset`은 repository가 소유하는 disposable volume 세 개만 삭제한다.
`JAMYE_CONFIRM_INFRA_RESET=jamye-server-test` 확인값이 필요하며 `.env.local`은
보존한다. Podman machine은 항상 사용자 소유이며 repository 명령은 이를 생성·시작·정지·
삭제하지 않는다.

MinIO app identity는 disposable administrator와 분리한다. media integration test에
필요한 최소 권한 policy는 `just minio-policy`로 연결한다.

## Process와 health

- `just run-api`는 `.env.local`을 읽어 API를 실행한다.
- `just run-worker`는 `.env.local`을 읽어 background worker를 실행한다.
- `just health-check`는 실행 중인 API의 `/health/live`와 PostgreSQL-backed readiness를
  확인한다.

구현 완료와 recovery gate는 [검증 절차](validation.md)를 참고한다.
