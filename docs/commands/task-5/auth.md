# M4 task-5 mobile authentication RED and dependency cards

## 목적

사용자가 승인한 D12=A를 task-5의 production mobile auth 경계로 고정한다. A1/A2는
system browser Authorization Code + PKCE S256을 사용하고 Redis에는 256-bit state의
digest와 10분 one-time attempt만 둔다. Task-5는 이 경계 위에서 hashed refresh-family
rotation/reuse fence, D13=A logout, shared Redis rate limit, U1/U2 profile을 구현한다.

Current task-5 provider allowlist는 Kakao와 Google이다. 사용자가 D4=A를 다시 확인해
Apple 실제 구현은 App Store 제출 전 별도 gate로 유지하며, 이메일/비밀번호 로그인은
현재 범위에 없다.

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

2026-08-25 사용자 실행은 locked graph compile 뒤 위 migration 부재 문구로 실패했고
`task_5_red_exit=101`을 반환했다. 따라서 task-5 TDD RED evidence는 유효하다.

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

Dependency delta는 다음으로 제한한다.

- 기존 optional `jsonwebtoken 11.0.0` + `aws_lc_rs`를 production Bearer verifier도
  공유하도록 기본 dependency로 승격하고 `dev-fixtures`는 dev surface만 제어한다.
- 이미 lock graph에 있는 `base64 0.22.1`을 PKCE S256 및 opaque credential의
  Base64URL no-padding 인코딩에 직접 사용한다.
- 기존 `reqwest 0.13.4`에 OAuth token form 전송과 JSON identity/JWKS decoding에
  필요한 `form`, `json` feature만 연다.
- HTTP, Redis, SHA-256, CSPRNG, URL, time dependency는 기존 선언을 재사용한다.

## Lock/no-drift

```bash
just --justfile scripts/tasks/task-5/mod.just locks
printf 'task_5_locks_exit=%s\n' "$?"
```

이 카드는 `Cargo.lock`을 resolver 결과로 갱신한 뒤 같은 명령을 다시 적용해
Cargo/Nix lock이 더 변하지 않는지 검증한다. crates.io index/download와 Nix evaluation
cache를 사용할 수 있지만 source, migration, database, service, credential은 변경하지
않는다. 실패하면 `Cargo.lock`을 손으로 고치거나 되돌리지 말고 전체 출력을 보존한다.

2026-08-25 사용자 실행은 `Cargo.lock` SHA-256
`41b5394336bf60aae75b91f5b1b987e994cb131ebd702610cdc81577a1077f91`, 기존
`flake.lock` SHA-256 `31403f6a698d7386579ca297f53952fd8cb47616affa8ff49c9fc71517f05bd9`를
고정했고 재실행 no-drift와 `task_5_locks_exit=0`을 확인했다. Lock delta는 direct
`base64`, Reqwest form/JSON의 `serde_urlencoded`와 root dependency edge뿐이며
미사용 RSA/ECDSA/EdDSA graph는 추가되지 않았다.

## Dependency policy

Lock/no-drift가 통과한 뒤에만 실행한다.

```bash
just --justfile scripts/tasks/task-5/mod.just dependency-check
printf 'task_5_dependency_exit=%s\n' "$?"
```

중복 crate warning은 정책 결과와 분리해 해석한다. `advisories`, `bans`, `licenses`,
`sources` 중 하나라도 실패하면 GREEN 구현을 시작하지 않고 원인 graph를 교정한다.

2026-08-25 사용자 실행은 기존 multi-version duplicate warning만 남긴 채
`advisories ok, bans ok, licenses ok, sources ok`와
`task_5_dependency_exit=0`을 반환했다. 따라서 task-5 production 구현 dependency gate는
열렸으며 별도 advisory 예외나 policy 완화는 없다.

Task-5 ADR은 기존 `0003` forward-only migration과 `0004` realtime 번호를 보존해
`0005-rate-limit-coordination.md`, `0006-mobile-oauth.md`를 사용한다. Migration은
forward-only `0002_auth_sessions.sql` 하나로 exact `0001` predecessor에서 검증한다.
