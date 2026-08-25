# M4 task-5 mobile authentication RED card

## 목적

사용자가 승인한 D12=A를 task-5의 production mobile auth 경계로 고정한다. A1/A2는
system browser Authorization Code + PKCE S256을 사용하고 Redis에는 256-bit state의
digest와 10분 one-time attempt만 둔다. Task-5는 이 경계 위에서 hashed refresh-family
rotation/reuse fence, D13=A logout, shared Redis rate limit, U1/U2 profile을 구현한다.

이번 RED는 production auth source, migration, DTO, provider adapter를 만들기 전에 최종
`tests/auth.rs` target이 정확히 task-5 구현 부재로 실패하는지 증명한다.

## 선행 조건

- task-3a, task-3b, task-3c가 완료돼 있어야 한다.
- D12=A와 D13=A가 사용자 승인 evidence로 locked돼 있어야 한다.
- `nix develop path:.` devShell 안이어야 한다.
- `migrations/0002_auth_sessions.sql`이 아직 없어야 한다.
- PostgreSQL, Redis, MinIO, API, worker나 OAuth provider credential은 필요하지 않다.

## RED

```bash
just --justfile scripts/tasks/task-5/mod.just red
printf 'task_5_red_exit=%s\n' "$?"
```

유효한 RED는 test binary가 현재 locked graph에서 정상 compile된 뒤 다음 원인으로
nonzero 종료한다.

```text
RED: migrations/0002_auth_sessions.sql is absent; task-5 must add D12=A PKCE OAuth, hashed refresh families, shared rate limiting, and profile surfaces
```

compile 오류, lockfile 변경, 다른 assertion 실패, 또는 migration이 이미 존재해 card가
exit `2`로 거절한 결과는 유효한 RED가 아니다. raw output을 전달한 뒤에만 auth/profile/
rate-limit behavior test와 production implementation을 작성한다.

## 부작용과 복구

- Cargo compile/test cache만 만들 수 있다.
- source, migration, lockfile, service, database, provider, Podman, Git state는 변경하지 않는다.
- OAuth state/code, access/refresh token, fixture row나 signing material을 만들지 않는다.
- 예상 문구가 아닌 실패는 첫 diagnostic을 그대로 보존한다. RED를 재현하려고 기존
  migration이나 source를 삭제하거나 Git 상태를 되돌리지 않는다.

## RED 이후 경계

유효한 RED 뒤 task-5가 실제 사용하는 최소 auth/crypto dependency만 검토한다.
`Cargo.lock`은 손으로 편집하지 않으며 사용자가 task-owned lock/no-drift와 dependency
audit card를 통과한 뒤에만 GREEN production 구현으로 진행한다.

Task-5 ADR은 기존 `0003` forward-only migration과 `0004` realtime 번호를 보존해
`0005-rate-limit-coordination.md`, `0006-mobile-oauth.md`를 사용한다. Migration은
forward-only `0002_auth_sessions.sql` 하나로 exact `0001` predecessor에서 검증한다.
