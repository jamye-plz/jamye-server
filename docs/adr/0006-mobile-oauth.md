# ADR 0006: Mobile Authorization Code with PKCE S256

- 상태: Accepted; D12=A와 D4=A/D16/D17 사용자 승인 반영
- 날짜: 2026-08-25; 2026-09-30 Apple native 경로 개정
- 범위: A1-A4 production mobile authentication, A6 Apple native exchange, U1/U2 profile identity, U3 Apple account deletion reauthentication

## Context

Legacy browser session cookie를 native app에 그대로 복사하면 system browser callback,
SecureStore, REST Bearer, refresh rotation과 realtime ticket 수명 경계를 명확히 소유할 수
없다. 사용자는 D12=A를 선택해 Authorization Code + PKCE S256을 승인했고, current
server/C2 provider-path OAuth는 Kakao와 Google로 한정했다. 2026-09-30 task-15 결정은
Apple을 provider-path OAuth가 아니라 iOS native Sign in with Apple의 identity token 검증
경로(A6)로 추가한다. 이메일/비밀번호 로그인은 현재 범위가 아니다.

## Decision

### A1 authorize

- App은 exact allowlist `redirect_uri`, 정확히 43자의 unpadded Base64URL SHA-256
  `code_challenge`, literal `S256`을 POST한다.
- Server는 OS CSPRNG로 각각 256-bit state와 nonce를 만들고 raw state는 app에 한 번
  반환한다. Redis key에는 state SHA-256 digest만 두며 value에는 provider, exact redirect,
  challenge, nonce만 600초 저장한다.
- Unknown provider와 provider-path `apple` 시도는 attempt 생성 전에
  `404 oauth_provider_not_supported`, configured-but-disabled Kakao/Google은
  `404 oauth_provider_not_available`을 반환한다. Apple 로그인은 A1/A2가 아니라 A6
  native identity-token endpoint만 사용한다.

### A2 exchange

- App callback exchange는 provider code, state, RFC 7636 verifier 43..128자, 처음과 같은
  redirect URI를 보낸다.
- Server는 state digest를 `GETDEL`로 먼저 소비한다. Provider/redirect/challenge binding이
  하나라도 틀리면 모두 `401 oauth_exchange_invalid`이고 attempt를 복구하지 않는다.
- Provider HTTPS I/O는 Redis/SQL transaction 밖에서 수행하며 bounded timeout과 redirect
  disabled client를 사용한다. Kakao/Google authorize, token, identity/JWKS origin은 source의
  official HTTPS constant이고 request/runtime config로 대체할 수 없다.
- Kakao는 immutable user `id`를 사용한다. Google은 JWKS signature, issuer, configured
  audience, expiry와 attempt nonce를 검증하고 userinfo `sub`가 ID token `sub`와 같은지
  확인한다.
- Provider access token, ID token, code와 client secret은 transient다. PostgreSQL, response
  diagnostics와 structured log에 저장하지 않는다.

### A6 Apple native exchange

- App은 iOS native Sign in with Apple 결과의 `identity_token`, raw nonce, 선택적
  `full_name`을 POST한다. Server는 Apple API를 호출하지 않고 JWKS signature, `kid`, `iss`,
  configured audience, expiry, nonce hash와 non-empty `sub`만 검증한다.
- 기능이 꺼져 있으면 `404 oauth_provider_not_supported`다. 검증 실패 원인은 public response에
  노출하지 않고 모두 `422 apple_identity_token_invalid`로 수렴시키며, 내부 tracing에도 token
  값은 남기지 않는다.
- Apple token, authorization code, client secret은 저장하지 않는다. A6에는 authorization code
  입력이 없고 `/auth/token` 호출도 없다.
- 신규 Apple 계정 nickname은 trimmed `full_name`이 1..64자면 사용하고, 아니면 기존 fallback
  규칙의 `Apple` + 6자리 숫자를 사용한다. 재로그인과 유예 계정 복구는 nickname/avatar를
  바꾸지 않는다.
- 유예 중 같은 Apple provider identity가 재로그인하면 기존 A2와 같이 계정을 복구하고
  `X-Jamye-Account-Restored: true`를 반환한다.

### U3 Apple account deletion reauthentication

- Apple 계정 삭제는 U3 JSON body에 `identity_token`, `authorization_code`, `raw_nonce`를 요구한다.
  Kakao/Google 계정은 U3 body를 계속 거부한다.
- Server는 먼저 identity token을 검증하고 `sub`가 저장된 Apple `provider_id`와 같은지 확인한다.
  그 뒤 SQL transaction 밖에서 client_secret(ES256)으로 Apple `/auth/token`을 호출하고,
  받은 token을 `/auth/revoke`로 폐기한 뒤 기존 30일 유예 삭제 transaction을 실행한다.
- 증명 누락은 `422 apple_reauthentication_required`, subject mismatch는
  `403 apple_subject_mismatch`, authorization code 오류는 `422 apple_authorization_code_invalid`,
  Apple 5xx/timeout은 `503 provider_unavailable`이다. revoke 전 실패는 DB mutation이 0이어야 한다.
  revoke 뒤 유예 transaction 실패는 `500 account_deletion_failed_after_revoke`로 보고한다.

### Identity, access, and refresh authority

- `auth_identities`의 `UNIQUE(provider, provider_id)`와 non-null user FK가 provider principal
  rebinding을 막는다. Concurrent callback은 transaction의 insert-or-load로 하나의 user와
  identity에 수렴하고 speculative loser user를 남기지 않는다.
- Exchange와 refresh 성공은 cookie 없이 정확한 five-field `TokenPair`를 반환한다. Access
  token은 production issuer/audience와 `sub`, `sid`, `iat`, `exp`를 서명·검증하며
  `jamye-dev` issuer를 production config에서 거부한다.
- Refresh credential은 256-bit opaque value를 응답에 한 번 반환하고 PostgreSQL에는
  SHA-256 digest만 저장한다. Parent row를 `FOR UPDATE`로 잠근 transaction 하나가 parent를
  consume하고 같은 family child를 삽입한다.
- Consumed parent 재사용은 family 전체를 revoke하고 `401 refresh_token_reused`를 반환한다.
  Unknown, expired, revoked token은 `401 refresh_token_invalid`다. Raw credential shape 오류는
  DB 조회 없이 `422 request_validation_failed`다.
- D13=A에서 logout은 bearer의 `sid`에 해당하는 refresh authority만 idempotently revoke한다.
  이미 발급된 짧은 access token은 서명된 `exp`까지 유효하고 unrelated session은 유지된다.
  Realtime ticket/socket deadline은 ADR 0004의 동일한 D13 경계를 따른다.

### Mobile handoff

Server repository는 executable SecureStore/single-flight simulator를 만들지 않는다. Task-5가
정적 two-send/one-refresh fixture를 단독 발행한다. 첫 ordinary 401은 두 outbox command와
원래 `client_msg_id`를 보존·pause하고 A3 한 요청에 합류한다. 성공하면 SecureStore
credential을 먼저 교체한 뒤 각 command를 같은 ID로 최대 한 번 replay한다. Invalid/reused
refresh 또는 replay의 두 번째 401은 credential을 지우고 loop 없이 재인증한다. 실제 실행
테스트는 `jamye-app`이 소유하고 task-12는 contribution을 final C2에 assemble/audit만 한다.

## Configuration and operational boundary

- Enabled provider는 complete client ID/secret과 wildcard/fragment 없는 exact redirect URI
  allowlist가 모두 있어야 한다. Incomplete enablement는 startup validation 실패다.
- Access signing secret, issuer, audience와 TTL은 secret-safe validated environment input이다.
  Provider URL이나 password auth를 환경변수로 켤 수 없다. Provider-path Apple slot은 계속
  지원하지 않으며, A6 native Apple 로그인만 `JAMYE_APPLE_SIGNIN_ENABLED=true`와 완전한
  `JAMYE_APPLE_*` secret set으로 켤 수 있다.
- Apple `.p8` private key는 PEM header/footer를 제거한 base64 PKCS#8 DER secret으로 받으며,
  PEM parser feature를 켜지 않고 `EncodingKey::from_ec_der`로 읽는다.
- `0002_auth_sessions.sql`은 exact `0001` predecessor에서 transactional upgrade하고 강제 실패
  시 전체 rollback한다. 이미 적용된 migration은 수정하지 않고 forward-fix를 새 번호로
  추가한다.
- Production credential 등록, callback URI console 변경, deployment, legacy identity import와
  App Store release gate는 별도 사용자 승인 범위다.

## Consequences

- 장점: system browser login과 native token storage가 cookie에 의존하지 않는다.
- 장점: state one-time consume, PKCE, exact redirect와 Google nonce가 callback을 attempt에
  결속한다.
- 장점: refresh reuse race가 server-side family fence로 닫혀 app single-flight 정확성에
  의존하지 않는다.
- 비용: Redis restart 뒤 시작했지만 끝나지 않은 OAuth attempt는 다시 A1부터 진행해야 한다.
- 비용: D13=A는 logout 직후 access token 즉시 폐기를 보장하지 않으며 최대 access TTL만큼
  기존 Bearer가 유효할 수 있다.

## References

- [Kakao Login REST API](https://developers.kakao.com/docs/en/kakaologin/rest-api)
- [Google OpenID Connect](https://developers.google.com/identity/openid-connect/openid-connect)
- [Google OAuth 2.0 for web server applications](https://developers.google.com/identity/protocols/oauth2/web-server)
- [Realtime ticket and D13 boundary](0004-realtime-ticket-storage.md)
- [검증 절차](../validation.md)
