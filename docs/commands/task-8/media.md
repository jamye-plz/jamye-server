# M7 task-8 private media platform cards

## 목적과 범위

Task-8은 D11=B로 고정된 private MinIO 경계와 task-4a의 caller-owned transaction,
task-6b history, task-7 topic promotion을 재사용해 다음 범위를 구현한다.

- DB-backed upload intent와 private presigned PUT/GET/download
- 실제 object HEAD MIME/size 검증과 audio container duration 검증
- chat confirmed/unbound capability와 topic atomic bind/finalize
- 최대 4개의 position-ordered message media와 bodyless exactly-one-audio voice
- cross-group BOLA/IDOR 방어, shared Redis presign rate limit, storage-only degradation
- exact `0005` predecessor에서 시작하는 forward-only `0006_media.sql`

D11=B에서 버킷 lifecycle owner는 object-storage adapter 하나다. Bucket HEAD 성공은
no-op, exact 404/`NoSuchBucket`만 scoped `CreateBucket`으로 생성하며, 나머지 provider
오류는 create로 오인하지 않고 typed degraded error로 반환한다.

미래 task-13은 homelab이 소비하는 NixOS module에서 native MinIO를 선택적으로 함께
실행할 수 있다. 이는 현재 task-8 구현 범위가 아니며 별도 bucket provisioning unit을
추가하지 않는다. 로컬 개발과 이 task의 integration harness는 rootless Podman Compose를
계속 사용한다.

## 선행 조건

- task-4a, task-6b, task-7 GREEN이 완료돼 있어야 한다.
- D11=B 사용자 선택이 machine plan과 roadmap에 locked evidence로 기록돼 있어야 한다.
- `nix develop path:.` devShell 안이어야 한다.
- RED 실행 시점에는 `migrations/0006_media.sql`과 task-8 production media surface가
  없어야 한다.
- RED에는 PostgreSQL, Redis, MinIO, API/worker 또는 container lifecycle이 필요하지 않는다.

## RED

```bash
just --justfile scripts/tasks/task-8/mod.just red
printf 'task_8_red_exit=%s\n' "$?"
```

유효한 RED는 `tests/media.rs`가 현재 locked dependency graph에서 정상 compile된 뒤
다음 원인으로 nonzero 종료한다.

```text
RED: migrations/0006_media.sql is absent; task-8 must add D11=B API-owned private-bucket lifecycle, authorized upload/finalize, ordered message media, and presigned access
```

예상 종료 코드는 Cargo test failure인 `101`이다. Compile 오류, lockfile 변경, 다른
assertion 실패, 또는 migration이 이미 존재해 card가 exit `2`로 거절한 결과는 유효한
RED가 아니다. Raw output을 전달한 뒤에만 dependency 선택, migration, behavior tests와
production implementation을 작성한다.

### 기록된 RED 증거

2026-08-26 사용자 실행에서 `tests/media.rs`가 현재 locked graph로 정상 compile된 뒤
위의 정확한 `0006_media.sql` 부재 문구로 실패했고 `task_8_red_exit=101`이 기록됐다.
따라서 task-8 TDD 구현 게이트는 열렸으며 dependency 제안·lock gate 준비로 진행한다.

## 부작용과 복구

- Cargo compile/test cache만 만들 수 있다.
- Source, migration, lockfile, PostgreSQL, Redis/MinIO state, container와 Git state는
  변경하지 않는다.
- 실패 출력은 첫 compile/error/assertion부터 그대로 보존한다.
- RED를 재현하려고 기존 source, volume, bucket 또는 object를 삭제하지 않는다.

## RED 이후 구현 경계

Task-8은 S3-compatible SDK와 media probing dependency의 Cargo ownership을 가진다. 유효한
RED 뒤 official-current compatibility와 실제 사용 feature만 확인한 dependency 제안을 먼저
제시하고, 사용자가 lock/no-drift와 dependency/license card를 실행한다.

### 선택한 dependency delta

- `aws-sdk-s3 = 1.144.0`: crate MSRV `1.94.1`은 repository Rust `1.98.0` 이하이다.
  MinIO에 필요한 custom endpoint, path-style addressing, SigV4 presign, bucket/object
  operations만 사용한다. `sigv4a`와 legacy `rustls` compatibility feature는 끄고
  `default-https-client`, `http-1x`, `rt-tokio`, `behavior-version-latest`만 명시한다.
  모든 object-storage 설정과 credential이
  feature-local config에서 명시되므로 별도 `aws-config` credential discovery는 추가하지 않는다.
- `symphonia = 0.6.1`: crate MSRV `1.85`는 repository toolchain 이하이다. Audio decode나
  SIMD가 아니라 실제 container timing만 필요하므로 default feature를 끄고 허용 MIME에
  대응하는 `isomp4`, `mkv`, `ogg` demuxer만 활성화한다.
- Symphonia family는 MPL-2.0이다. 전역 license allowlist를 넓히지 않고 실제 lock graph에
  필요한 정확한 `0.6.1` 일곱 crate만 `deny.toml` exception으로 제한한다.

첫 사용자 lock 실행은 exit `0`으로 Cargo/Nix lock 검증을 통과했고 `flake.lock`이
그대로임을 확인했다. Lock review에서 불필요한 AWS legacy Hyper 0.14/Rustls 0.21 graph를
유발한 `rustls` compatibility feature와 누락된 `symphonia-common` MPL exception을 발견해
위와 같이 범위를 더 좁히고 정확히 보완했다. 따라서 아래 lock card는 수정된 최종 manifest로
다시 실행해야 한다.

수정 후 사용자 재실행도 exit `0`으로 통과했다. 최종 graph는 333 packages,
`Cargo.lock` SHA-256은 `62507c613de017b2b0184646d9f7c0eaa9d74efb66be9ff9a2a573bc9ab40167`,
`flake.lock` SHA-256은 기존
`31403f6a698d7386579ca297f53952fd8cb47616affa8ff49c9fc71517f05bd9` 그대로다.
Hyper 0.14/Rustls 0.21 transport family가 제거됐고, exact Symphonia 0.6.1 seven-crate
MPL exception set이 lock graph와 일치한다.

## Dependency lock

```bash
just --justfile scripts/tasks/task-8/mod.just locks
printf 'task_8_locks_exit=%s\n' "$?"
```

이 card는 task-1의 lock create/verify primitive를 그대로 재사용한다. `Cargo.lock`은 위의
두 직접 dependency와 필요한 transitive graph만 반영해야 하고, `flake.lock`은 바뀌지 않아야
한다. 종료 코드 `0`과 두 lockfile의 검증 결과를 전달한 뒤 dependency policy를 확인한다.

## Dependency policy

```bash
just --justfile scripts/tasks/task-8/mod.just dependency-check
printf 'task_8_dependency_exit=%s\n' "$?"
```

유효한 결과는 advisory, ban, license, source policy가 모두 통과하고 종료 코드가 `0`인
경우다. Duplicate-version warning은 경로를 함께 검토하되 이 repository의 ban policy가
warning으로 분류한 것만 gate를 막지 않는다.

### 기록된 dependency evidence

2026-08-26 사용자 실행에서 최종 333-package lock graph를 대상으로 policy card가
`advisories ok, bans ok, licenses ok, sources ok`를 모두 출력하고
`task_8_dependency_exit=0`으로 종료했다. 출력된 duplicate-version 항목은 configured
warning이며, AWS SDK와 기존 SQLx/Axum/Reqwest 계보가 요구하는 서로 다른 major/minor
버전임을 경로와 함께 확인했다. 따라서 task-8 dependency gate는 완료됐다.

이후 D11=B HEAD/no-op·404/create·non-404 typed-error를 가장 먼저 behavior RED/GREEN으로
고정한다. 그 다음 `0006` migration, upload/finalize/binding transaction, ordered projection,
HTTP/contract contribution, disposable-MinIO security/recovery를 순서대로 구현한다.

## D11=B bucket lifecycle RED

```bash
just --justfile scripts/tasks/task-8/mod.just bucket-red
printf 'task_8_bucket_red_exit=%s\n' "$?"
```

이 card는 외부 MinIO나 container를 사용하지 않는다. 저수준 bucket backend fake로 다음 세
분기를 동시에 compile하고 실행한다.

- HEAD 성공은 `Existing` no-op이며 create를 호출하지 않는다.
- 정확한 `Missing`만 create를 한 번 호출하고 `Created`를 반환한다.
- non-missing provider 오류는 같은 typed 원인을 보존하며 create를 호출하지 않는다.

현재 production adapter는 의도적으로 `Provider(Unavailable)`만 반환하는 RED scaffold다.
유효한 RED는 세 테스트가 모두 발견·compile된 뒤 위 기대값과 다른 assertion으로 실패하고
`task_8_bucket_red_exit=101`을 기록하는 것이다. Compile 오류, lockfile drift, 외부 서비스
오류, 테스트 미발견은 유효한 RED가 아니다.

### 기록된 bucket RED 증거

2026-08-26 사용자 실행에서 새 AWS/Symphonia graph와 production crate가 정상 compile됐고,
세 bucket lifecycle 테스트가 모두 발견됐다. 세 테스트는 의도된 RED scaffold의
`Err(Provider(Unavailable))`와 `Existing`·`Created`·preserved `AccessDenied` 기대값의
assertion 차이로만 실패했으며 `task_8_bucket_red_exit=101`을 기록했다.

## D11=B bucket lifecycle GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just bucket-green
printf 'task_8_bucket_green_exit=%s\n' "$?"
```

GREEN은 HEAD 성공 시 no-op, `Missing`일 때만 create-once, 다른 provider 오류의 typed 보존을
모두 통과해 3 passed와 exit `0`을 기록해야 한다. 이 card 역시 외부 서비스나 container를
사용하지 않는다.

### 기록된 bucket GREEN 증거

2026-08-26 사용자 실행에서 세 bucket lifecycle 테스트가 모두 통과하고
`task_8_bucket_green_exit=0`을 기록했다. 기존 task-5 `SensitiveValue` dead-code 경고 두 개
외에 새 경고나 실패는 없었다.

## Object-storage configuration RED

```bash
just --justfile scripts/tasks/task-8/mod.just config-red
printf 'task_8_config_red_exit=%s\n' "$?"
```

이 card는 외부 서비스 없이 feature-local 설정만 검증한다. Production은 완전한 여섯 값과
외부 HTTPS public presign endpoint를 요구하고, 내부 service endpoint만 loopback HTTP를
허용한다. Test/development는 명시된 loopback HTTP 구성을 허용하되 부분 설정을 거부하며,
모두 미설정이면 process가 아니라 storage 기능만 degraded 상태가 된다. Credential은 Debug나
오류에 노출하지 않는다.

현재 resolver는 의도적으로 항상 `None`인 compile-valid RED scaffold다. 유효한 RED는 7개
테스트가 모두 발견·compile되고, 설정된 값 또는 rejection을 기대한 assertion에서 실패해
exit `101`을 기록하는 것이다. `nonproduction_all_absent...` 한 테스트만 통과할 수 있다.

### 기록된 configuration RED 증거

2026-08-26 사용자 실행에서 7개 테스트가 모두 compile·발견됐다. 완전 미설정
nonproduction 한 테스트만 통과하고, 나머지 6개는 의도된 `None` scaffold와 configured 또는
rejected 결과의 차이로 실패했으며 `task_8_config_red_exit=101`을 기록했다.

## Object-storage configuration GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just config-green
printf 'task_8_config_green_exit=%s\n' "$?"
```

GREEN은 7 passed와 exit `0`을 기록해야 한다. 이 단계는 config resolver 자체를 검증하며,
process composition과 실제 S3 client 연결은 후속 adapter/integration gate에서 검증한다.

### 기록된 configuration GREEN 증거

2026-08-26 사용자 실행에서 7개 설정 테스트가 모두 통과하고
`task_8_config_green_exit=0`을 기록했다. 기존 task-5 `SensitiveValue` dead-code 경고 두 개
외에 새 경고나 실패는 없었다.

## Concrete S3 SDK boundary RED

```bash
just --justfile scripts/tasks/task-8/mod.just sdk-red
printf 'task_8_sdk_red_exit=%s\n' "$?"
```

이 card는 Podman이나 실제 MinIO를 사용하지 않는다. 프로세스 내부 임시 HTTP 서버가
`200`, `404 -> 200`, `403` 응답을 제공하고, production `S3BucketBackend`가 명시적
feature-local credential과 internal endpoint로 다음 동작을 하는지 검증한다.

- path-style private bucket `HEAD`를 SigV4로 서명한다.
- `200`은 create 없는 `Existing`이다.
- 정확한 `404`만 한 번의 signed `PUT CreateBucket`으로 이어진다.
- `403`은 `AccessDenied`로 보존되고 create를 호출하지 않는다.
- Authorization header 어디에도 raw secret access key가 포함되지 않는다.

RED scaffold의 concrete backend는 SDK client를 안전하게 구성하되 두 operation 모두
의도적으로 `Unavailable`을 반환했다. 유효한 RED는 세 테스트가 모두
발견·compile된 뒤 `Existing`·`Created`·`AccessDenied` 기대와 scaffold 결과의 assertion
차이로 실패하고 `task_8_sdk_red_exit=101`을 기록하는 것이다. Compile 오류, lockfile drift,
실제 서비스 연결 오류, 테스트 미발견은 유효한 RED가 아니다.

### 기록된 concrete SDK RED 증거

2026-08-26 사용자 실행에서 production crate와 세 SDK boundary 테스트가 정상 compile됐다.
세 테스트는 각각 scaffold의 `Err(Provider(Unavailable))`와 `Existing`·`Created`·typed
`AccessDenied` 기대값의 assertion 차이로만 실패했고 `task_8_sdk_red_exit=101`을 기록했다.
설정 panic, lock drift, 임시 HTTP server 또는 외부 service 오류는 없었다.

## Concrete S3 SDK boundary GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just sdk-green
printf 'task_8_sdk_green_exit=%s\n' "$?"
```

GREEN은 세 SDK boundary 테스트가 모두 통과하고 exit `0`을 기록해야 한다. 실제 disposable
MinIO에서 identity/policy/private-bucket 및 stop/restart를 검증하는 integration gate는
media transaction과 HTTP surface가 완성된 뒤 별도로 실행한다.

첫 GREEN 실행에서는 세 operation 결과와 typed 분류, method, SigV4 서명, secret 비노출이
모두 기대대로였지만 AWS SDK가 bucket-root URI를 `/{bucket}/`으로 canonicalize한 데 비해
test fixture가 `/{bucket}`을 기대해 세 assertion이 실패했다. Production 동작은 수정하지 않고
관측된 SDK canonical path에 맞춰 fixture 기대값만 고쳤다. 재실행 exit `0`이 최종 GREEN이다.

### 기록된 concrete SDK GREEN 증거

2026-08-26 사용자 재실행에서 세 SDK boundary 테스트가 모두 통과했고
`task_8_sdk_green_exit=0`을 기록했다. HEAD no-op, 정확한 404의 create-once, 403의 typed
`AccessDenied`, path-style URI, SigV4 서명과 raw secret 비노출이 모두 확인됐다. 기존
task-5 `SensitiveValue` dead-code 경고 두 개 외에 새 경고나 실패는 없었다.

## Media schema RED

```bash
just --justfile scripts/tasks/task-8/mod.just migration-red
printf 'task_8_migration_red_exit=%s\n' "$?"
```

이 card는 PostgreSQL이나 MinIO를 시작하지 않고 전체 migration test module이 compile되는지
확인한 뒤, 정확히 한 static contract test만 실행한다. 유효한 RED는
`RED: migrations/0006_media.sql is absent` 원인으로 실패하고 exit `101`을 기록해야 한다.
Compile 오류, 다른 테스트 실패, migration이 이미 존재해 생긴 실패는 유효한 RED가 아니다.

RED가 고정하는 다음 GREEN 범위는 exact `0005` predecessor upgrade, 강제 실패 전체 rollback,
`media_uploads`의 chat/topic 소비자 shape, `message_media`의 upload 단일 소비와 position
`0..3`, `topic_media.media_upload_id` 단일 소비다. 계획 JSON의 `bound_message_id unique`는
동일 계획의 메시지당 1–4개 첨부와 양립하지 않으므로 메시지 ID 자체는 unique로 만들지 않는다.
대신 upload ID와 bound message의 결합 FK, 첨부의 `UNIQUE(media_upload_id)` 및 위치 unique로
한 업로드의 재사용은 막고 한 메시지의 네 첨부는 허용한다.

### 기록된 media schema RED 증거

2026-08-26 사용자 실행에서 전체 media test binary와 네 migration test가 정상 compile됐다.
선택된 static test는 `migrations/0006_media.sql` 부재만 보고 실패했고
`task_8_migration_red_exit=101`을 기록했다. PostgreSQL 연결, migration SQL assertion 또는
다른 테스트 실패는 없었다.

## Media schema GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just migration-green
printf 'task_8_migration_green_exit=%s\n' "$?"
```

이 card는 guarded local test 환경을 불러와 disposable PostgreSQL만 사용한다. GREEN은
exact `0005`에서 `0006`으로 업그레이드하고, 강제 실패가 두 새 relation과
`topic_media.media_upload_id`를 모두 rollback하며, 동일 메시지의 position `0..3` 네 첨부는
허용하되 다섯 번째 위치와 한 upload의 chat/topic 교차 소비는 거절해야 한다. 정상적인
topic upload와 `topic_media`의 양방향 deferred binding도 같은 transaction에서 commit돼야 한다.

### 기록된 media schema GREEN 증거

2026-08-26 사용자 실행에서 네 migration 테스트가 모두 통과했고
`task_8_migration_green_exit=0`을 기록했다. Exact `0005` predecessor upgrade, 강제 실패 시
두 relation과 topic binding 전체 rollback, position `0..3` 네 첨부 허용과 다섯 번째 거절,
chat/topic 교차 소비 거절 및 정상 topic 양방향 deferred binding commit이 disposable
PostgreSQL에서 확인됐다. 기존 auth dead-code 경고와 test support의 미사용 `pool` 경고 외에
새 실패는 없었다.

MD1 concrete adapter RED에서 shared filename policy와 0006의 불일치를 발견해 두 filename
column/check를 빈 문자열 허용·Unicode 최대 255 characters로 정렬한 뒤 같은 card를 다시
실행했다. 재실행에서도 네 migration 테스트가 모두 통과하고
`task_8_migration_green_exit=0`을 기록했으며 compile 경고도 없었다.

## Shared upload policy RED

```bash
just --justfile scripts/tasks/task-8/mod.just policy-red
printf 'task_8_policy_red_exit=%s\n' "$?"
```

이 card는 PostgreSQL, Redis, MinIO 없이 presign과 finalize가 함께 소비할 framework-free
정책을 고정한다. Chat은 정확한 이미지 4종(10 MiB), `video/mp4`(50 MiB), 오디오
WebM/MP4/Ogg(15 MiB)를 허용하고 topic은 이미지 4종만 허용한다. 크기는 1 byte 이상 각
종류 cap 이하이며 MIME은 대소문자 변경, 매개변수 또는 공백을 정규화하지 않는다. 선택적
파일명은 Unicode character 기준 최대 255자를 원문 그대로 보존하고, object key는 client
path 입력 없이 서버가 `chat/{target_id}/{upload_id}` 또는
`topics/{target_id}/{upload_id}`로 만든다. PUT/GET TTL 3600/600초, audio 330초, 메시지당
최대 4개도 같은 테스트가 고정한다.

현재 production 함수는 compile-valid RED scaffold다. 유효한 RED는 다섯 테스트를 모두
발견·compile한 뒤 constants 테스트 하나만 통과하고, upload validation 세 테스트와 key
minting 테스트가 scaffold의 `UnsupportedContentType` 또는 placeholder key 차이로 실패해
`task_8_policy_red_exit=101`을 기록해야 한다. Compile 오류, lockfile drift, 서비스 연결
오류 또는 다른 module 실패는 유효한 RED가 아니다.

### 기록된 shared upload policy RED 증거

2026-08-26 사용자 실행에서 다섯 정책 테스트가 모두 정상 compile·발견됐다. Locked constants
테스트 하나는 통과했고, chat/topic/filename 검증 세 테스트는 scaffold의 정확한
`UnsupportedContentType` 차이로, key 테스트는 `task-8-policy-red` placeholder 차이로만
실패했다. `task_8_policy_red_exit=101`이 기록됐으며 lock drift나 외부 서비스 오류는 없었다.

## Shared upload policy GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just policy-green
printf 'task_8_policy_green_exit=%s\n' "$?"
```

GREEN은 같은 다섯 테스트가 모두 통과하고 exit `0`을 기록해야 한다. 이 정책은 이후 MD1
upload-intent 생성과 MD2 실제 object HEAD finalize 양쪽에서 직접 재사용하며, 어느 한쪽이
별도 MIME/크기 표를 갖지 않는다.

### 기록된 shared upload policy GREEN 증거

2026-08-26 사용자 실행에서 다섯 정책 테스트가 모두 통과했고
`task_8_policy_green_exit=0`을 기록했다. Exact allowlist와 per-kind cap, topic image-only,
Unicode filename 255-character 경계, server-minted chat/topic key, PUT/GET TTL과 audio/message
limits가 service-free test로 확인됐다. RED 뒤 테스트·상수·허용 목록은 변경하지 않았다.

## MD1 upload intent orchestration RED

```bash
just --justfile scripts/tasks/task-8/mod.just upload-intent-red
printf 'task_8_upload_intent_red_exit=%s\n' "$?"
```

이 card는 PostgreSQL, Redis, MinIO 없이 fake port와 기존 opaque transaction handle로 MD1의
호출 순서를 고정한다. Invalid MIME/size/filename은 limiter 이전에 종료해야 한다. Valid
request는 user+scope+target으로 격리된 `media_upload_presign` subject를 shared limiter에 먼저
확인하고, 허용된 경우에만 transaction을 열어 membership+target 검사를 포함한 intent insert,
short PUT presign, 한 번의 commit 순서로 진행한다. Limiter denial/outage는 transaction·insert·
presign 0회이며, target authorization 또는 presign 실패는 commit 없이 rollback해야 한다.

현재 use case는 compile-valid RED scaffold로 `ObjectStorageDegraded`만 반환한다. 유효한 RED는
네 테스트가 모두 발견·compile된 뒤 request validation, rate-limit fail-closed, 성공 call
order, rollback assertion이 scaffold 결과와 달라 실패하고
`task_8_upload_intent_red_exit=101`을 기록해야 한다. Compile 오류, lockfile drift 또는 외부
서비스 연결은 유효한 RED가 아니다.

### 기록된 MD1 upload intent orchestration RED 증거

2026-08-26 사용자 실행에서 네 orchestration 테스트와 production crate가 정상 compile됐다.
네 테스트는 각각 scaffold의 `ObjectStorageDegraded`와 request validation, rate-limit result,
successful outcome, target authorization 기대값의 차이로만 실패했고
`task_8_upload_intent_red_exit=101`을 기록했다. Transaction handle, fake port, lockfile 또는
외부 서비스 오류는 없었다.

## MD1 upload intent orchestration GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just upload-intent-green
printf 'task_8_upload_intent_green_exit=%s\n' "$?"
```

GREEN은 같은 네 테스트가 모두 통과하고 exit `0`을 기록해야 한다. 이 gate 뒤 실제
PostgreSQL membership/target insert와 AWS SDK presign은 별도 integration RED/GREEN으로
검증한다.

### 기록된 MD1 upload intent orchestration GREEN 증거

2026-08-26 사용자 실행에서 네 orchestration 테스트가 모두 통과했고
`task_8_upload_intent_green_exit=0`을 기록했다. Shared policy가 limiter보다 먼저 실행되고,
limiter denial/outage는 transaction과 storage side effect 없이 종료되며, 성공 경로는
rate-limit → begin → authorized insert → presign → 단일 commit 순서를 지켰다. Target
authorization 또는 presign 실패는 commit 없이 rollback했다. 이번 slice에서 새로 드러난
`PresignedPut` 미사용 import는 제거했으며 기존 auth dead-code와 test-support `pool` 경고는
task-8 변경과 무관하다.

## MD1 concrete adapters RED

```bash
just --justfile scripts/tasks/task-8/mod.just upload-intent-adapters-red
printf 'task_8_upload_intent_adapters_red_exit=%s\n' "$?"
```

이 card는 guarded disposable PostgreSQL과 네트워크 전송 없는 AWS SDK presign builder를
사용한다. PostgreSQL test는 live membership과 실제 chat/topic target을 같은 caller-owned
transaction 안에서 검사하고, commit된 pending intent만 남으며 outsider·없는 target·명시적
rollback은 row를 남기지 않는 경계를 고정한다. 선택적 filename은 legacy/public 정책과 같이
빈 문자열 및 최대 255 Unicode characters를 저장할 수 있어야 한다.

SDK test는 internal endpoint와 다른 public endpoint를 넣고, 반환 PUT URL이 public origin,
private bucket의 path-style key, 3600초 TTL, SigV4 credential scope와 signature를 사용하며
content-length/content-type/host를 서명에 결속하는지 검증한다. Raw secret은 URL에 없어야 하며
presign 생성 자체는 HTTP request를 보내지 않는다.

현재 두 production adapter는 compile-valid RED scaffold로 각각 `Unavailable`을 반환한다.
유효한 RED는 두 테스트가 모두 발견·compile된 뒤 PostgreSQL success 기대와 SDK presign
success 기대에서만 실패하고 `task_8_upload_intent_adapters_red_exit=101`을 기록해야 한다.
Migration/setup 오류, lock drift, 외부 S3 연결 또는 테스트 미발견은 유효한 RED가 아니다.

### 기록된 MD1 concrete adapters RED 증거

2026-08-26 사용자 실행에서 두 adapter integration 테스트와 production crate가 정상
compile됐다. PostgreSQL 테스트는 stub의 `Unavailable` 때문에, SDK 테스트도 같은 typed
stub 결과 때문에 각각 실패했으며 `task_8_upload_intent_adapters_red_exit=101`을 기록했다.
Disposable database setup/migration, lockfile, 외부 S3 연결 또는 테스트 발견 오류는 없었다.
이 RED가 정책과 migration 사이의 filename 불일치도 드러냈다. Shared policy의 빈 문자열 허용과
Unicode 최대 255 characters를 authoritative하게 유지하도록 0006의 두 filename column/check를
같은 경계로 정렬한다.

## MD1 concrete adapters GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just upload-intent-adapters-green
printf 'task_8_upload_intent_adapters_green_exit=%s\n' "$?"
```

GREEN은 같은 두 테스트가 모두 통과하고 exit `0`을 기록해야 한다. PostgreSQL adapter는
transaction을 시작하거나 끝내지 않고 전달받은 handle만 사용하며, SDK adapter는 validated
public endpoint와 명시적 credential만 사용한다. 이 card의 production compile에서는 task-12
연결 전 임시 `#[expect(dead_code)]`가 task-5 auth secret 경고 두 개를 정확히 억제하는지도
함께 확인한다. Adapter GREEN 뒤 0006 filename 경계 변경을 포함한 migration GREEN도 다시
실행한다.

### 기록된 MD1 concrete adapters GREEN 증거

2026-08-26 사용자 실행에서 두 concrete adapter 테스트가 모두 통과하고
`task_8_upload_intent_adapters_green_exit=0`을 기록했다. PostgreSQL adapter는 live
membership과 chat/topic target을 caller-owned transaction에서 검사하고 commit된 두 pending
intent만 남겼으며, outsider·없는 target·명시적 rollback은 row를 남기지 않았다. 빈 filename과
255-character Unicode filename도 정책 그대로 저장됐다. AWS SDK는 HTTP 전송 없이 public
path-style origin의 3600초 PUT URL을 만들고 content-length/content-type/host를 SigV4 서명에
결속했으며 raw secret을 노출하지 않았다. Production compile 출력에는 경고가 없어서 task-12
전 임시 auth `#[expect(dead_code)]` 두 개도 정확히 작동함을 확인했다.

## MD2 authoritative object finalize policy RED

```bash
just --justfile scripts/tasks/task-8/mod.just finalize-policy-red
printf 'task_8_finalize_policy_red_exit=%s\n' "$?"
```

이 card는 PostgreSQL, Redis, MinIO 없이 presign 때 검증한 upload policy와 provider/object
probe가 실제로 관측한 metadata의 결합 규칙을 고정한다. Content-Type과 byte size는 누락 없이
intent 값과 정확히 일치해야 하며 MIME normalization이나 client 값 대체는 없다. Image/video는
duration을 저장하지 않는다. Audio는 실제 container duration이 필요하고 `0 < duration <= 330s`
여야 한다. Fractional duration은 저장 시 올림해 실제 길이를 축소 기록하지 않는다.

현재 validator는 compile-valid RED scaffold로 항상 `MetadataMissing`을 반환한다. 유효한 RED는
세 테스트가 모두 발견·compile된 뒤 exact image/video 성공, MIME/size typed mismatch, audio
duration 성공/경계 기대가 scaffold 결과와 달라 실패하고
`task_8_finalize_policy_red_exit=101`을 기록해야 한다. 외부 서비스 오류, lock drift 또는
테스트 미발견은 유효한 RED가 아니다.

### 기록된 MD2 authoritative object finalize policy RED 증거

2026-08-26 사용자 실행에서 세 테스트와 production crate가 경고 없이 정상 compile됐다.
세 테스트는 exact image/video 성공, MIME typed mismatch, audio fractional/cap 성공 기대와
scaffold의 `MetadataMissing` 결과 차이로만 모두 실패했고
`task_8_finalize_policy_red_exit=101`을 기록했다. 외부 서비스, lockfile 또는 테스트 발견
오류는 없었다.

## MD2 authoritative object finalize policy GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just finalize-policy-green
printf 'task_8_finalize_policy_green_exit=%s\n' "$?"
```

GREEN은 같은 세 테스트가 모두 통과하고 exit `0`을 기록해야 한다. 이 policy 뒤 별도
orchestration RED/GREEN이 authorization → object HEAD/probe → caller-owned transaction finalize
순서와 rollback/idempotency를 고정하고, concrete adapter gate가 실제 AWS SDK와 Symphonia
container probe를 검증한다.

### 기록된 MD2 authoritative object finalize policy GREEN 증거

2026-08-26 사용자 실행에서 세 테스트가 모두 통과하고
`task_8_finalize_policy_green_exit=0`을 기록했다. Exact image/video metadata, MIME/size 불일치,
audio duration 필수·양수·330초 cap·fractional 올림 규칙이 모두 통과했으며 production crate와
테스트 출력에는 경고가 없었다.

## MD2 finalize orchestration RED

```bash
just --justfile scripts/tasks/task-8/mod.just finalize-orchestration-red
printf 'task_8_finalize_orchestration_red_exit=%s\n' "$?"
```

이 service-free card는 upload owner/target authorization을 object access보다 먼저 수행하고,
실제 object HEAD/container probe가 끝날 때까지 PostgreSQL transaction을 열지 않는 순서를
고정한다. Chat 성공은 하나의 transaction으로 upload를 `confirmed` 처리한 뒤 unbound 결과를
반환한다. Topic 성공은 같은 caller-owned handle에서 upload consume + `topic_media` bind 뒤
task-7의 ordinary `promote_enriched`를 호출하고 한 번만 commit한다. Exact retry는 canonical
결과를 그대로 반환하며 object I/O나 새 transaction을 만들지 않는다. Metadata/provider,
finalize write, topic promotion 실패는 부분 mutation이나 commit 없이 끝나야 한다.

현재 application method는 compile-valid RED scaffold로 storage-degraded 오류만 반환한다.
유효한 RED는 여섯 orchestration 테스트와 production crate가 정상 compile된 뒤, 위 결과·순서
기대와 scaffold의 차이로 실패하고 exit `101`을 기록해야 한다. Compile 오류, 테스트 미발견,
외부 서비스 연결 오류는 유효한 RED가 아니다.

### 기록된 MD2 finalize orchestration RED 증거

2026-08-26 사용자 실행에서 여섯 orchestration 테스트와 production crate가 경고 없이 정상
compile됐다. Authorization/conflict, object/metadata pre-transaction failure, chat confirmed,
topic bind+promotion, rollback, canonical retry 기대가 모두 storage-degraded scaffold 결과와
달라 6/6 실패했고 `task_8_finalize_orchestration_red_exit=101`을 기록했다. 외부 서비스,
lockfile 또는 테스트 발견 오류는 없었다.

## MD2 finalize orchestration GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just finalize-orchestration-green
printf 'task_8_finalize_orchestration_green_exit=%s\n' "$?"
```

GREEN은 같은 여섯 테스트가 모두 통과하고 exit `0`을 기록해야 한다. Authorization과 exact
retry read는 transaction 밖에서 수행되고, pending object inspection/validation 뒤에만 caller-owned
transaction을 연다. Chat은 finalize 뒤 한 번 commit하고, topic은 같은 handle에서 finalize →
ordinary `promote_enriched` 뒤 한 번 commit한다. Finalize/promotion 실패는 rollback하며 provider
metadata나 policy 실패는 transaction 자체를 만들지 않는다.

### 기록된 MD2 finalize orchestration GREEN 증거

2026-08-26 사용자 실행에서 같은 여섯 orchestration 테스트가 모두 통과하고
`task_8_finalize_orchestration_green_exit=0`을 기록했다. Authorization/conflict의 object I/O 전
종료, object/metadata 실패의 transaction 전 종료, chat 단일 commit, topic finalize → ordinary
`promote_enriched` → 단일 commit, 실패 rollback, exact retry의 canonical no-I/O 반환이 모두
확인됐으며 production crate와 테스트 출력에는 경고가 없었다.

## MD2 concrete finalize adapters RED

```bash
just --justfile scripts/tasks/task-8/mod.just finalize-adapters-red
printf 'task_8_finalize_adapters_red_exit=%s\n' "$?"
```

이 card는 guarded disposable PostgreSQL과 process-local scripted S3만 사용한다. PostgreSQL
테스트는 upload owner와 chat target membership, topic author 또는 group owner 권한을 먼저
확인하고, expired/다른 actor/ordinary topic member를 mutation 없이 거부하는지 고정한다.
Chat finalize는 caller rollback 뒤 `pending`, commit 뒤 canonical `confirmed` retry여야 한다.
Topic finalize는 같은 handle에서 upload bind + `topic_media` insert + ordinary
`promote_enriched`를 수행하며, rollback은 세 상태를 모두 원복하고 commit/retry는 같은 canonical
row 하나만 반환해야 한다.

S3 테스트는 public presign origin과 다른 internal endpoint로 path-style object `HEAD`가
SigV4 서명되는지, provider의 exact MIME/Content-Length만 반환하는지 확인한다. Image에는 GET을
호출하지 않는다. Audio에는 HEAD 뒤 signed GET을 한 번 호출하며, 테스트가 결정적으로 생성한
최소 Ogg/Opus container로 seekable header duration과 end-header가 없는 packet-duration fallback을
모두 10ms로 검증한다. Fixture는 외부 MinIO, ffmpeg, binary asset 또는 network에 의존하지 않는다.

현재 PostgreSQL `prepare_upload_finalize`/`finalize_upload`와 S3 `inspect_object`는 production
compile-valid RED scaffold로 `Unavailable`만 반환한다. 유효한 RED는 다섯 concrete adapter
테스트와 production crate가 정상 compile된 뒤 이 세 stub 결과 때문에 다섯 테스트가 실패하고
`task_8_finalize_adapters_red_exit=101`을 기록해야 한다. Migration/setup, lock drift, 외부 S3
연결, 테스트 미발견 또는 compile 오류는 유효한 RED가 아니다.

### 기록된 MD2 concrete finalize adapters RED 증거

- 실행일: 2026-08-26
- production crate와 다섯 concrete adapter 테스트가 정상 compile되었다.
- PostgreSQL prepare/finalize 및 S3 inspect scaffold의 `Unavailable` 때문에 의도한 다섯 테스트가
  모두 실패했다(0 passed, 5 failed, 38 filtered out).
- migration/setup, lock drift, 외부 S3 연결, 테스트 미발견, compile 오류 및 경고는 없었다.
- 기록된 종료 코드: `task_8_finalize_adapters_red_exit=101`

## MD2 concrete finalize adapters GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just finalize-adapters-green
printf 'task_8_finalize_adapters_green_exit=%s\n' "$?"
```

GREEN은 같은 다섯 테스트가 모두 통과하고 exit `0`을 기록해야 한다. PostgreSQL prepare는
upload owner와 현재 target 권한을 함께 확인하되 object I/O 동안 lock을 보유하지 않는다.
Finalize는 caller transaction에서 같은 권한을 다시 확인하고 upload row 및 권한 근거를 잠근 뒤
chat confirm 또는 topic bind/`topic_media` insert를 수행한다. Topic promotion은 이 adapter 밖의
ordinary topics repository 호출이지만 같은 transaction handle을 사용한다. S3 PUT presign은 public
origin을 유지하고, object HEAD/GET은 internal origin에서 path-style SigV4 요청을 사용한다. Audio는
HEAD가 허용된 MIME·크기를 확인한 경우에만 bounded body를 읽어 container header duration을 우선
사용하고, header duration이 없으면 packet duration을 합산하되 codec decode는 하지 않는다.

첫 GREEN 시도는 소유된 `Uuid` 역참조와 SQLx 0.9가 거부한 동적 SQL 문자열 때문에 production
crate compile 단계에서 중단됐으므로 GREEN 증거로 인정하지 않았다. 역참조를 제거하고 upload
잠금 조회를 정적 SQL로 바꾼 뒤 같은 card를 다시 실행했다.

### 기록된 MD2 concrete finalize adapters GREEN 증거

2026-08-26 사용자 재실행에서 production crate가 경고 없이 정상 compile됐고, 다섯 concrete
adapter 테스트가 모두 통과해 `task_8_finalize_adapters_green_exit=0`을 기록했다. Internal signed
path-style HEAD와 authoritative metadata, audio header/packet duration 경로, PostgreSQL owner/target
authorization, caller rollback과 canonical retry, topic bind/insert/promotion의 동일 atomic handle
사용이 disposable PostgreSQL 및 process-local scripted S3에서 확인됐다.

## C4 message/media composition policy RED

```bash
just --justfile scripts/tasks/task-8/mod.just message-policy-red
printf 'task_8_message_policy_red_exit=%s\n' "$?"
```

이 card는 PostgreSQL, Redis, MinIO 및 HTTP 없이 framework-free domain policy만 검증한다.
Missing/null/empty body와 빈 media의 조합은 정확히 `MessageContentRequired`로 거절하고,
whitespace를 포함한 비어 있지 않은 body는 그대로 허용한다. Visual media는 요청 순서를
position `0..3`으로 보존하며 최대 네 개까지 허용하고, 다섯 번째 attachment와 duplicate
upload ID/object key는 side effect 전에 거절한다. Audio가 포함되면 bodyless exactly-one-audio만
voice로 허용하고 text+audio, multiple audio 및 audio/visual mixing은 거절한다.

현재 validator는 의도적으로 모든 입력을 `MessageContentRequired`로 반환하는 compile-valid RED
scaffold다. 유효한 RED는 네 테스트가 모두 발견·compile되고 빈 content 거절 한 테스트만 통과한
뒤 나머지 세 테스트가 기대한 accepted/typed 결과와 scaffold의 차이로 실패해
`task_8_message_policy_red_exit=101`을 기록하는 것이다. Compile 오류, 테스트 미발견 또는 다른
module 실패는 유효한 RED가 아니다.

### 기록된 C4 message/media composition policy RED 증거

2026-08-26 사용자 실행에서 production crate와 네 policy 테스트가 경고 없이 정상 compile됐다.
Missing/null/empty body와 빈 media를 거절하는 한 테스트는 통과했고, 나머지 세 테스트는 의도된
`MessageContentRequired` scaffold와 ordinary visual media, typed count/duplicate error 및 voice
composition 기대의 assertion 차이로만 실패해 `task_8_message_policy_red_exit=101`을 기록했다.
외부 service, lockfile, 테스트 발견 또는 compile 오류는 없었다.

## C4 message/media composition policy GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just message-policy-green
printf 'task_8_message_policy_green_exit=%s\n' "$?"
```

GREEN은 같은 네 테스트가 모두 통과하고 exit `0`을 기록해야 한다. 이 정책은 후속 message-media
binding이 caller-owned transaction을 열거나 upload row/object를 조회하기 전에 실행하며, 요청의
media 순서만 authoritative position으로 사용하고 client가 object key나 position을 주입할 surface를
만들지 않는다.

### 기록된 C4 message/media composition policy GREEN 증거

2026-08-26 사용자 실행에서 production crate가 경고 없이 정상 compile됐고, 네 policy 테스트가
모두 통과해 `task_8_message_policy_green_exit=0`을 기록했다. Missing/null/empty body 경계,
whitespace 보존, 최대 네 개 visual attachment의 요청 순서 `0..3`, duplicate upload/key 거절,
bodyless exactly-one-audio voice 및 audio mixing 거절이 모두 확인됐다.

## C4 concrete PostgreSQL message-media binding RED

```bash
just --justfile scripts/tasks/task-8/mod.just message-binding-adapters-red
printf 'task_8_message_binding_adapters_red_exit=%s\n' "$?"
```

이 card는 task-1의 loopback disposable PostgreSQL guard를 그대로 사용한다. Production port는
요청 DTO가 제공하는 upload ID와 provider에서 다시 관측한 metadata만 받으며 object key,
filename, position은 command에 존재하지 않는다. Concrete adapter가 caller-owned transaction에서
message/actor/chatroom 권한과 confirmed capability를 다시 잠그고 확인한 뒤 DB 소유 metadata를
복사하는 계약을 다섯 테스트로 고정한다.

- visual attachment 네 개는 request order를 정확한 position `0..3`으로 사용한다.
- bodyless exactly-one-audio는 저장된 authoritative duration을 보존한다.
- 다른 actor/target/message, 다른 owner의 capability, pending/expired capability는 typed하게
  거절되고 아무 row도 소비하지 않는다.
- caller rollback은 `message_media`와 upload bound/consumed 상태를 모두 되돌린다.
- 동일 message/upload/order/metadata retry는 기존 canonical attachment를 반환하고, 순서 변경이나
  다른 message의 재사용은 conflict이며 중복 row를 만들지 않는다.
- bind 직전에 다시 관측한 kind/MIME/size/audio duration이 confirmed metadata와 다르면 conflict로
  거절한다.

현재 `PostgresMediaRepository::bind_message_media`는 의도적으로 `Unavailable`을 반환하는
compile-valid RED scaffold다. 유효한 RED는 production crate와 다섯 테스트가 모두 compile·발견된
뒤 각 테스트의 첫 정상 binding 기대가 scaffold와 달라 0 passed, 5 failed 및 exit `101`을
기록하는 것이다. Migration/setup 실패, lock drift, 테스트 미발견, compile 오류 또는 경고는
유효한 RED가 아니다.

첫 RED 시도는 test helper와 로컬 변수에 모두 `command`라는 이름을 사용해 retry case의 뒤쪽
helper 호출 두 곳이 가려졌고, `E0618` compile 오류로 중단됐다. 따라서 exit `101`이더라도 유효한
RED 증거로 인정하지 않는다. Helper를 `binding_command`로 명확히 바꾼 뒤 같은 card를 다시
실행한다.

### 기록된 C4 concrete PostgreSQL message-media binding RED 증거

2026-08-27 사용자 재실행에서 production crate가 경고 없이 정상 compile됐고, 다섯 concrete
adapter 테스트가 모두 발견됐다. 각 테스트는 의도된 `Unavailable` scaffold가 첫 정상 binding을
거절한 지점에서만 실패해 0 passed, 5 failed 및
`task_8_message_binding_adapters_red_exit=101`을 기록했다. Disposable PostgreSQL setup,
lockfile, 테스트 발견 및 compile 오류는 없었으므로 유효한 behavioral RED다.

## C4 concrete PostgreSQL message-media binding GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just message-binding-adapters-green
printf 'task_8_message_binding_adapters_green_exit=%s\n' "$?"
```

GREEN은 같은 다섯 테스트가 모두 통과하고 exit `0`을 기록해야 한다. Repository는 transaction을
begin/commit하지 않고 전달받은 task-4a handle만 사용한다. Exact retry 이외의 partial match는
항상 conflict이며, DB constraint와 row lock이 한 upload의 message/topic 교차 소비를 함께 막는다.

### 기록된 C4 concrete PostgreSQL message-media binding GREEN 증거

2026-08-27 사용자 실행에서 production crate가 경고 없이 정상 compile됐고, 다섯 concrete
adapter 테스트가 모두 통과해 `task_8_message_binding_adapters_green_exit=0`을 기록했다.
DB 소유 metadata의 request-order 복사, bodyless exactly-one-audio, actor/target/capability 재검증,
caller rollback, exact canonical retry, 순서 변경·교차 소비·provider metadata drift conflict가
disposable PostgreSQL에서 확인됐다.

## MD5 download Content-Disposition policy RED

```bash
just --justfile scripts/tasks/task-8/mod.just access-policy-red
printf 'task_8_access_policy_red_exit=%s\n' "$?"
```

이 card는 PostgreSQL, Redis, MinIO 및 HTTP 없이 다운로드 response override 문자열만 검증한다.
저장 filename이 없으면 exact allowlist MIME extension으로 `jamye-{media_id}.{ext|bin}`을 만들고,
Korean/Unicode filename은 ASCII fallback과 RFC 5987 `filename*=UTF-8''...`를 함께 제공한다.
저장 filename에 CR/LF, quote, path separator 또는 traversal이 포함돼도 header parameter나 경로로
탈출할 수 없으며 object key는 filename 입력 surface가 아니다.

현재 `download_content_disposition`은 의도적으로 `attachment`만 반환하는 compile-valid RED
scaffold다. 유효한 RED는 세 테스트가 모두 발견·compile된 뒤 exact fallback, Unicode encoding,
unsafe-character sanitization 기대와 scaffold의 assertion 차이로 0 passed, 3 failed 및 exit `101`을
기록하는 것이다. 외부 service 접근, lockfile 변경, compile 오류, 경고 또는 테스트 미발견은
유효한 RED가 아니다.

### 기록된 MD5 download Content-Disposition policy RED 증거

2026-08-27 사용자 실행에서 production crate와 세 policy 테스트가 경고 없이 정상 compile됐고,
모두 발견됐다. 세 테스트는 의도된 `attachment` scaffold와 MIME fallback, RFC 5987 Unicode,
unsafe path/header sanitization 기대의 assertion 차이로만 실패해 0 passed, 3 failed 및
`task_8_access_policy_red_exit=101`을 기록했다. 외부 service 접근, lockfile 변경, compile 오류
또는 테스트 미발견은 없었으므로 유효한 behavioral RED다.

## MD5 download Content-Disposition policy GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just access-policy-green
printf 'task_8_access_policy_green_exit=%s\n' "$?"
```

GREEN은 같은 세 테스트가 모두 통과하고 exit `0`을 기록해야 한다. 이 단계가 만드는 문자열은
후속 MD5 application/object-storage adapter가 S3 `response-content-disposition`에 그대로 bind하며,
raw presigned URL이나 filename을 log 또는 public persistence에 새로 남기지 않는다.

### 기록된 MD5 download Content-Disposition policy GREEN 증거

2026-08-27 사용자 실행에서 production crate가 경고 없이 정상 확인됐고, 세 policy 테스트가
모두 통과해 `task_8_access_policy_green_exit=0`을 기록했다. Allowlisted MIME fallback,
Unicode RFC 5987 `filename*`, ASCII fallback 및 path/header/traversal sanitization이 service-free
경계에서 고정됐다.

## MD4/MD5 authorized GET-presign orchestration RED

```bash
just --justfile scripts/tasks/task-8/mod.just access-orchestration-red
printf 'task_8_access_orchestration_red_exit=%s\n' "$?"
```

이 card는 PostgreSQL, MinIO, Redis 및 HTTP 없이 application 호출 순서를 검증한다. MD4 view와
MD5 download 모두 `actor_id + media_id`만 repository에 전달하고, repository가 authoritative
message/topic relation으로 권한과 DB-owned metadata/object key를 확정한 뒤에만 exact 600초 GET을
presign해야 한다. View는 response override를 넣지 않고, download는 앞 단계에서 고정한 safe
Content-Disposition만 bind한다. 공개 `MediaAccessUrl`에는 object key가 없다.

Repository 권한 실패는 object storage 호출 전에 `TargetNotAccessible`로 끝나야 하며, 권한 확인
뒤 presign 실패만 `ObjectStorageDegraded`로 분류한다. 이 read 경계는 PostgreSQL transaction이나
lock을 외부 I/O 동안 보유하지 않는다.

현재 `MediaAccessService`의 두 method는 compile-valid RED scaffold로 항상
`ObjectStorageDegraded`를 반환한다. 유효한 RED는 네 테스트가 모두 발견·compile된 뒤 결과와
`Authorize -> Presign` 호출 순서 assertion에서 0 passed, 4 failed 및 exit `101`을 기록하는 것이다.
Compile 오류, lockfile 변경, 외부 service 접근, 테스트 미발견 또는 다른 경고는 유효한 RED가 아니다.

### 기록된 MD4/MD5 authorized GET-presign orchestration RED 증거

2026-08-27 사용자 실행에서 production crate와 네 orchestration 테스트가 경고 없이 정상
compile됐고 모두 발견됐다. 세 테스트는 scaffold의 `ObjectStorageDegraded`와 authorization 또는
successful presign 기대 결과가 달라 실패했고, storage failure 테스트는 결과 분류는 같지만
`Authorize -> Presign` 호출이 전혀 없어서 실패했다. 따라서 0 passed, 4 failed 및
`task_8_access_orchestration_red_exit=101`은 의도된 behavioral RED이며 외부 service, lock drift,
compile 오류 또는 테스트 미발견은 없었다.

## MD4/MD5 authorized GET-presign orchestration GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just access-orchestration-green
printf 'task_8_access_orchestration_green_exit=%s\n' "$?"
```

GREEN은 같은 네 테스트가 모두 통과하고 exit `0`을 기록해야 한다. 이후 concrete PostgreSQL
authorization query와 public-origin S3 GET presign은 별도의 adapter RED/GREEN에서 검증한다.

### 기록된 MD4/MD5 authorized GET-presign orchestration GREEN 증거

2026-08-27 사용자 실행에서 production crate가 경고 없이 정상 compile됐고, 네 orchestration
테스트가 모두 통과해 `task_8_access_orchestration_green_exit=0`을 기록했다. Repository BOLA
실패가 storage 전에 중단되고, 성공 시에만 DB-owned key로 exact 600초 GET을 presign하며, view는
override 없음, download는 safe Content-Disposition bind, storage 실패는 authorization 뒤
`ObjectStorageDegraded`로 분류되는 호출 순서가 고정됐다.

## MD4/MD5 concrete access adapters RED

```bash
just --justfile scripts/tasks/task-8/mod.just access-adapters-red
printf 'task_8_access_adapters_red_exit=%s\n' "$?"
```

이 card는 task-1의 loopback disposable PostgreSQL guard를 재사용하고, S3-compatible server나
HTTP listener는 시작하지 않는다. PostgreSQL 테스트는 `actor_id + media_id`만 받아 authoritative
`message_media -> messages -> chatrooms -> groups -> memberships` 또는
`topic_media -> topics -> groups -> memberships` relation으로 권한을 판정한다. 성공 결과의
object key, MIME, size, dimensions, duration, filename은 모두 DB-owned 값이어야 한다. 다른 그룹
actor와 존재하지 않는 ID는 relation 종류나 존재 여부를 드러내지 않고 동일한
`TargetNotAccessible`로 거절한다.

두 SDK 테스트는 실제 `S3MediaObjectStorage`가 전송 없이 만드는 presigned GET을 검사한다. URL은
internal endpoint가 아니라 public path-style origin을 사용하고 exact 600초 TTL과 host SigV4
signature를 가져야 한다. View에는 response override가 없으며, download에는 policy 단계에서 만든
safe Content-Disposition이 `response-content-disposition`으로 정확히 결속돼야 한다. Raw secret은
URL에 노출되지 않는다.

현재 concrete PostgreSQL repository와 S3 object-storage adapter는 두 port의 default
`Unavailable` 구현을 그대로 사용한다. 유효한 RED는 production crate와 세 테스트가 정상
compile·발견된 뒤 PostgreSQL 성공 기대 한 건과 SDK GET 성공 기대 두 건이 이 stub 때문에
0 passed, 3 failed 및 exit `101`을 기록하는 것이다. Migration/setup 실패, lock drift, 외부 S3
연결, 테스트 미발견, compile 오류 또는 경고는 유효한 RED가 아니다.

### 첫 concrete access adapters RED 시도 — compile 오류로 무효

2026-08-27 사용자 실행은 `tests/media/access_adapters.rs`의 PostgreSQL fixture 정리 코드에서
`E0382`로 compile이 중단됐다. Expected record 두 개를 fixture에서 소유권 이동한 다음
`fixture.dispose(self)`로 전체 fixture를 다시 소비하려 한 partial move가 원인이었다. 따라서
exit `101`이더라도 세 adapter 테스트가 발견되지 않았으므로 behavioral RED 증거로 인정하지
않는다. Cleanup을 실패 assertion보다 먼저 수행하는 구조는 유지하고, expected record만 clone해
fixture 전체를 dispose할 수 있도록 최소 수정한 뒤 같은 RED card를 다시 실행한다.

### 기록된 MD4/MD5 concrete access adapters RED 증거

2026-08-27 사용자 재실행에서 production crate와 세 concrete adapter 테스트가 경고 없이 정상
compile됐고 모두 발견됐다. PostgreSQL 테스트는 concrete repository의 `Unavailable` 때문에 첫
authorized chat record 기대에서 실패했고, 두 SDK 테스트도 concrete object-storage adapter의
동일한 default `Unavailable` 때문에 실패했다. Disposable PostgreSQL setup/migration은 정상
완료됐고 외부 S3 연결이나 lock drift는 없었다. 따라서 0 passed, 3 failed 및
`task_8_access_adapters_red_exit=101`은 의도된 behavioral RED다.

## MD4/MD5 concrete access adapters GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just access-adapters-green
printf 'task_8_access_adapters_green_exit=%s\n' "$?"
```

GREEN은 같은 세 테스트가 모두 통과하고 exit `0`을 기록해야 한다. PostgreSQL 조회는 attachment
relation과 현재 group membership을 한 query에서 결합하고 필요한 DB-owned field만 반환한다.
GET presign은 public client만 사용하며 repository authorization과 외부 object I/O 사이에
transaction이나 row lock을 유지하지 않는다.

### 기록된 MD4/MD5 concrete access adapters GREEN 증거

2026-08-27 사용자 실행에서 production crate가 경고 없이 compile됐고 세 concrete adapter 테스트가
모두 통과했다. PostgreSQL adapter는 message/topic attachment relation과 현재 group membership을
기준으로 authorized record를 반환하면서 cross-group 및 missing relation을 거부했다. SDK view와
download presign은 public path-style origin, exact 600초 TTL, 안전한 Content-Disposition 결속 및
secret 비노출 조건을 만족했다. 결과는 3 passed, 0 failed 및
`task_8_access_adapters_green_exit=0`이다.

## MD4/MD5 authenticated HTTP access RED

```bash
just --justfile scripts/tasks/task-8/mod.just access-http-red
printf 'task_8_access_http_red_exit=%s\n' "$?"
```

이 card는 PostgreSQL, Redis, MinIO 및 listener 없이 Axum router와 fake application port만
사용한다. 두 endpoint 모두 Bearer 인증 뒤 path의 `media_id`를 UUID로 검증해야 한다. MD4는
object key를 제외한 DB-owned metadata와 exact 600초 short URL을 JSON으로 반환한다. MD5는 body
없는 `307 Temporary Redirect`와 authorized signed URL `Location`만 반환한다. Missing/cross-group
media는 같은 `403 media_not_accessible`, object-storage 장애는 exact
`503 object_storage_degraded`, PostgreSQL 장애는 exact `503 database_unavailable` envelope를
사용하며 `details`는 항상 null이다.

현재 compile-valid router scaffold는 인증 extractor와 stable error taxonomy만 소유하고 모든
인증된 요청을 `media_unavailable`로 거부한다. 유효한 RED는 다섯 테스트가 정상 compile·발견된
뒤 이 scaffold 때문에 0 passed, 5 failed 및 exit `101`을 기록하는 것이다. 인증 누락 검증은 첫
테스트 내부에서 먼저 통과하지만 malformed UUID assertion이 실패해야 한다. 테스트 미발견,
compile 오류, warning, 외부 서비스 연결 또는 lock drift는 유효한 RED가 아니다.

### 기록된 MD4/MD5 authenticated HTTP access RED 증거

2026-08-27 사용자 실행에서 production crate와 다섯 HTTP 테스트가 경고 없이 정상 compile되고
모두 발견됐다. 인증 누락 검증은 먼저 통과했으며, 나머지는 compile-valid router scaffold가 모든
인증된 요청을 `503 media_unavailable`로 거부해 MD4의 `200`, MD5의 빈 `307`, malformed UUID의
`422 request_validation_failed`, BOLA의 `403 media_not_accessible`, storage/database의 구분된
stable `503` 기대와 정확히 어긋났다. 외부 service, listener, lock drift 또는 테스트 미발견은
없었다. 따라서 0 passed, 5 failed 및 `task_8_access_http_red_exit=101`은 의도된 behavioral RED다.

## MD4/MD5 authenticated HTTP access GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just access-http-green
printf 'task_8_access_http_green_exit=%s\n' "$?"
```

GREEN은 같은 다섯 테스트가 모두 통과하고 exit `0`을 기록해야 한다. Handler는 UUID parsing과
application 호출 및 public response projection만 담당하며 authorization/presign 순서와
Content-Disposition 생성은 이미 검증된 `MediaAccessService`에 그대로 위임한다. 성공/실패 log는
request ID와 stable code만 기록하고 raw presigned URL이나 object key를 남기지 않는다.

### 기록된 MD4/MD5 authenticated HTTP access GREEN 증거

2026-08-27 사용자 실행에서 production crate가 경고 없이 compile됐고 다섯 HTTP 테스트가 모두
통과했다. Bearer 인증과 malformed UUID 검증, MD4의 public-safe metadata 및 exact 600초 URL,
MD5의 body 없는 `307 Location`, non-disclosing BOLA envelope, storage/database별 stable `503`이
고정됐다. 결과는 5 passed, 0 failed 및 `task_8_access_http_green_exit=0`이다.

## MD1/MD2 authenticated HTTP RED

```bash
just --justfile scripts/tasks/task-8/mod.just upload-finalize-http-red
printf 'task_8_upload_finalize_http_red_exit=%s\n' "$?"
```

이 card는 PostgreSQL, Redis, MinIO 및 listener 없이 Axum router와 fake application dependency만
사용한다. MD1은 Bearer 인증 뒤 strict `UploadIntentCreate`를 받아 `201`로 server-minted intent와
content-type/size/TTL이 결속된 `PresignedPut`을 반환해야 한다. MD2는 strict UUID path와 optional
positive width/height만 받아 chat이면 confirmed/unbound capability, topic이면 bound TopicMedia와
`topic_status=enriched`를 같은 discriminated response로 반환해야 한다.

여섯 테스트는 인증 및 malformed input, MD1 성공 projection, rate-limit `Retry-After`와 stable
dependency/BOLA 오류, MD2 chat variant, MD2 topic variant, finalize conflict/BOLA/validation/dependency
오류를 고정한다. 현재 compile-valid mutation-router scaffold는 인증 extractor와 공통 stable error
envelope만 소유하고 인증된 요청을 `503 media_unavailable`로 거부한다. 유효한 RED는 production
crate와 여섯 테스트가 경고 없이 compile·발견된 뒤 이 scaffold 차이로 0 passed, 6 failed 및 exit
`101`을 기록하는 것이다. 테스트 미발견, compile 오류, 외부 service 연결 또는 lock drift는
유효한 RED가 아니다.

### 기록된 MD1/MD2 authenticated HTTP RED 증거

2026-08-27 사용자 실행에서 production crate와 여섯 HTTP 테스트가 경고 없이 정상 compile되고
모두 발견됐다. 인증 누락 검증은 먼저 통과했으며, 나머지는 compile-valid mutation-router
scaffold가 인증된 요청을 `503 media_unavailable`로 거부해 MD1의 `201`, rate-limit
`Retry-After`, MD2 chat/topic의 `200`, malformed input의 `422`, BOLA/conflict/validation 및
dependency별 stable envelope 기대와 정확히 어긋났다. 외부 service, listener, lock drift 또는
테스트 미발견은 없었다. 따라서 0 passed, 6 failed 및
`task_8_upload_finalize_http_red_exit=101`은 의도된 behavioral RED다.

## MD1/MD2 authenticated HTTP GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just upload-finalize-http-green
printf 'task_8_upload_finalize_http_green_exit=%s\n' "$?"
```

GREEN은 같은 여섯 테스트가 모두 통과하고 exit `0`을 기록해야 한다. Handler는 body/path 검증,
authenticated actor 전달, public DTO projection 및 stable HTTP 변환만 담당한다. Upload policy,
rate-limit, authorization, provider inspection, transaction, retry 및 topic promotion은 이미 검증된
`MediaService`와 `MediaFinalizeService`에 위임하고 raw presigned URL/object key/credential은 log에
남기지 않는다.

### 기록된 MD1/MD2 authenticated HTTP GREEN 증거

2026-08-27 사용자 실행에서 production crate가 경고 없이 compile됐고 여섯 HTTP 테스트가 모두
통과했다. Bearer 인증과 strict body/path 검증, MD1의 server-minted intent 및 constrained PUT,
rate-limit `Retry-After`, MD2 chat confirmed/unbound 및 topic bound/enriched projection, 그리고
BOLA/conflict/validation/storage/database별 stable envelope가 고정됐다. 결과는 6 passed,
0 failed 및 `task_8_upload_finalize_http_green_exit=0`이다.

## MD3/history media read projections RED

```bash
just --justfile scripts/tasks/task-8/mod.just read-projections-red
printf 'task_8_read_projections_red_exit=%s\n' "$?"
```

이 card는 Redis, MinIO 및 listener 없이 disposable PostgreSQL과 실제 topic/chatroom query adapter,
인증된 HTTP router를 사용한다. History는 `message_media.position` 순서대로 공개-safe attachment를
반환하고 object key는 노출하지 않아야 한다. MD3 topic detail은 task-8의 one-time binding identity인
canonical `media_upload_id`를 기존 TopicMedia metadata와 함께 반환해야 한다.

현재 compile-valid query는 history의 `media`를 항상 빈 배열로 만들고, task-7 TopicMedia projection은
`media_upload_id`를 읽거나 직렬화하지 않는다. 유효한 RED는 두 테스트와 production crate가 경고
없이 compile·발견된 뒤 history의 기대 2개 대비 실제 0개, MD3의 기대 upload UUID 대비 누락된 null로
각각 실패해 0 passed, 2 failed 및 exit `101`을 기록하는 것이다. Compile 오류, fixture/migration
오류, 테스트 미발견 또는 외부 object-storage 연결은 유효한 RED가 아니다.

### 기록된 MD3/history media read projections RED 증거

2026-08-27 사용자 실행에서 production crate가 경고 없이 compile됐고 두 PostgreSQL-backed HTTP
테스트가 모두 발견됐다. History는 저장된 두 attachment 대비 실제 빈 배열 길이 `0`으로, canonical
topic media는 기대 upload UUID 대비 누락된 `Null`로만 실패했다. 결과는 0 passed, 2 failed 및
`task_8_read_projections_red_exit=101`이며 Redis, MinIO 또는 listener 실패는 없었다.

## MD3/history media read projections GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just read-projections-green
printf 'task_8_read_projections_green_exit=%s\n' "$?"
```

GREEN은 같은 두 테스트가 모두 통과하고 exit `0`을 기록해야 한다. History query는 먼저 기존
membership-safe page를 확정한 뒤 해당 message ID 집합의 attachment만 한 번에 조회해 position
순서로 결합하며 pagination과 sender projection을 바꾸지 않는다. Topic get/list/transactional retry
projection은 `topic_media.media_upload_id`를 canonical DTO에 추가한다. 어느 공개 history attachment도
private object key를 포함하지 않는다.

### 기록된 MD3/history media read projections GREEN 증거

2026-08-27 사용자 실행에서 production crate가 경고 없이 compile됐고 두 PostgreSQL-backed HTTP
테스트가 모두 통과했다. History는 저장된 attachment를 `message_media.position` 순서대로 공개-safe
metadata로 반환하면서 private object key를 노출하지 않았고, 기존 topic detail projection은 canonical
`media_upload_id`를 포함했다. 결과는 2 passed, 0 failed, 73 filtered out 및
`task_8_read_projections_green_exit=0`이다.

이 GREEN은 C2 history projection과 기존 T4 topic detail 안의 task-8 media identity 확장을 고정한다.
기계 SSOT의 별도 MD3 `GET /api/v1/topics/{topic_id}/media` paginated collection 계약은 다음 TDD
slice에서 독립적으로 검증한다.

## MD3 paginated topic-media HTTP RED

```bash
just --justfile scripts/tasks/task-8/mod.just md3-http-red
printf 'task_8_md3_http_red_exit=%s\n' "$?"
```

이 card는 disposable PostgreSQL과 실제 Topics router/service/repository adapter를 사용해 기계 SSOT의
별도 MD3 `GET /api/v1/topics/{topic_id}/media`를 검증한다. 성공 응답은 `TopicMediaPage`의
`items,next_cursor`만 포함하고, item은 `(created_at,id)` 오름차순으로 안정 정렬된 canonical
`media_upload_id`, topic metadata 및 기존 public `object_key`를 반환한다. `after+limit`은 page 사이에
중복이나 누락을 만들지 않아야 한다.

Bearer 부재는 `401 authentication_required`, malformed path/cursor, duplicate/unknown query 및
`limit=0|101`은 `422 request_validation_failed`다. Missing topic, nonmember topic 및 다른 topic의
cursor는 존재 여부나 media metadata를 누출하지 않는 동일한 `403 membership_required` envelope를
사용한다.

현재 production router에는 이 경로가 없으므로 compile-valid RED는 세 테스트가 모두 발견된 뒤
기대 `200|401|403` 대비 실제 `404` route miss로만 실패하고 exit `101`을 기록해야 한다. Compile
오류, fixture/migration 오류, 테스트 미발견 또는 외부 object-storage 연결은 유효한 RED가 아니다.

### 기록된 MD3 paginated topic-media HTTP RED 증거

2026-08-27 사용자 실행에서 production crate와 세 PostgreSQL-backed HTTP 테스트가 경고 없이
compile·발견됐다. 성공 page, Bearer auth/validation 및 non-disclosing BOLA 테스트가 각각 기대
`200`, `401`, `403` 대비 아직 등록되지 않은 경로의 `404`로만 실패했다. 결과는 0 passed,
3 failed, 75 filtered out 및 `task_8_md3_http_red_exit=101`이며 fixture/migration 또는 외부
object-storage 오류는 없었다.

## MD3 paginated topic-media HTTP GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just md3-http-green
printf 'task_8_md3_http_green_exit=%s\n' "$?"
```

GREEN은 같은 세 테스트가 모두 통과하고 exit `0`을 기록해야 한다. Router는 auth/path/query parsing과
public DTO 변환만 담당하고, service가 page input을 검증하며, PostgreSQL adapter가 topic membership,
cursor scope 및 `limit + 1` keyset page를 한 query shape로 확정한다.

### 기록된 MD3 paginated topic-media HTTP GREEN 증거

2026-08-27 사용자 실행에서 production crate가 경고 없이 compile됐고 세 PostgreSQL-backed HTTP
테스트가 모두 통과했다. Bearer 인증과 strict path/query 검증, `(created_at,id)` 오름차순 keyset
pagination, canonical `media_upload_id` projection, topic-scoped cursor 및 missing/nonmember/cross-topic
cursor의 동일한 non-disclosing `403 membership_required` envelope가 고정됐다. 결과는 3 passed,
0 failed, 75 filtered out 및 `task_8_md3_http_green_exit=0`이다.

## Media contract contribution RED

```bash
just --justfile scripts/tasks/task-8/mod.just contract-red
printf 'task_8_contract_red_exit=%s\n' "$?"
```

이 card는 PostgreSQL, Redis, MinIO 및 listener 없이 Task-8의 feature-local contract contribution을
정적으로 검증한다. 세 테스트는 MD1–MD5의 정확한 operation inventory, MD2의 scope discriminator,
public-safe `MessageAttachment`/`MediaAccessUrl`, 기존 C2 history의 최대 4개 ordered attachment,
T4/MD3의 canonical `media_upload_id`, 그리고 bodyless exactly-one-audio voice의
finalize→unchanged-client_msg_id send/retry→message.created/delta/history→MD4/MD5 reissue fixture를
고정한다.

현재 `contracts/contributions/task-8/`은 없고 task-6b/task-7 contribution은 각각 `media: []`와
`media_upload_id` 이전 shape다. 유효한 RED는 production crate와 세 테스트가 경고 없이
compile·발견된 뒤 새 contribution 파일 부재 두 건과 기존 history/topic wire assertion 차이로
0 passed, 3 failed 및 exit `101`을 기록하는 것이다. Compile 오류, 외부 service 연결, generated
C0/C2 snapshot 직접 수정 또는 테스트 미발견은 유효한 RED가 아니다.

### 기록된 media contract contribution RED 증거

2026-08-27 사용자 실행에서 production crate와 세 정적 contract 테스트가 경고 없이
compile·발견됐다. 두 테스트는 각각 task-8 `media-flow.json`과 `operations.json` 부재의 정확한
RED 문구로 실패했고, 나머지 테스트는 기존 task-6b history schema의 `maxItems: 0`과 기대값 `4`의
assertion 차이로 실패했다. 결과는 0 passed, 3 failed, 78 filtered out 및
`task_8_contract_red_exit=101`이며 외부 service 또는 generated snapshot 변경은 없었다.

## Media contract contribution GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just contract-green
printf 'task_8_contract_green_exit=%s\n' "$?"
```

GREEN은 같은 세 테스트가 모두 통과하고 exit `0`을 기록해야 한다. Task-8은 feature-local
operation/schema/fixture contribution과 task-6b/task-7의 media extension만 소유한다. 최종 42-operation
C2 snapshot 생성과 production composition은 기계 SSOT대로 task-12에 남기며, 현재 generated
`contracts/openapi.json`, realtime snapshot 및 C0 fixture를 이 card에서 다시 생성하지 않는다.

### 기록된 media contract contribution GREEN 증거

2026-08-27 사용자 실행에서 production crate와 세 정적 contract 테스트가 경고 없이 compile됐고
모두 통과했다. MD1–MD5의 정확한 operation inventory, MD2 scope discriminator, public-safe
attachment/access DTO, 최대 4개의 position-ordered history attachment, canonical topic
`media_upload_id`, 그리고 voice finalize→send/retry→realtime/delta/history→reissue flow가 함께
고정됐다. 결과는 3 passed, 0 failed, 78 filtered out 및
`task_8_contract_green_exit=0`이다.

## Disposable MinIO least-privilege boundary RED

먼저 task-1의 disposable infrastructure가 실행 중이어야 한다. 이 card 자체는 container,
identity, policy, bucket 또는 object를 생성·수정·삭제하지 않으며 loopback MinIO health만
preflight한다.

```bash
just --justfile scripts/tasks/task-8/mod.just minio-boundary-red
printf 'task_8_minio_boundary_red_exit=%s\n' "$?"
```

정적 테스트는 아직 없는 `scripts/tasks/task-8/minio-app-policy.json`이 정확히 선택된
`jamye-task8-media` bucket lifecycle과 private object GET/PUT만 허용하고 wildcard/admin/list-all/
delete 권한은 주지 않아야 함을 고정한다. 실제 MinIO 테스트는 task-1이 만든 서로 다른
admin/bootstrap 및 policy 없는 app identity를 사용한다. App identity가 API-owned
`ensure_bucket`으로 선택 bucket만 만들고 sibling bucket 생성은 거절되며, signed PUT/HEAD/GET은
성공하되 anonymous list/head/get은 모두 `403`이어야 한다. Debug, public URL, anonymous 응답에는
admin credential과 secret/password가 없어야 한다.

현재 유효한 RED는 production crate와 두 테스트가 compile·발견된 뒤 다음 두 구현 공백으로
실패하는 것이다.

- task-8 MinIO app policy 파일 부재의 정확한 `RED:` 문구
- policy 없는 app identity의 `ensure_bucket` 결과가 기대한 `Created`가 아니라 typed
  `AccessDenied`인 assertion 차이

예상 결과는 0 passed, 2 failed 및 exit `101`이다. MinIO health preflight 실패, loopback/test
guard의 exit `2`, compile 오류, admin credential 사용 또는 기존 bucket/policy가 개입한 결과는
유효한 RED가 아니다.

### 기록된 disposable MinIO boundary RED 증거

2026-08-27 사용자 실행에서 production crate와 두 MinIO boundary 테스트가 경고 없이
compile·발견됐다. 정적 테스트는 `scripts/tasks/task-8/minio-app-policy.json` 부재의 정확한
`RED:` 문구로 실패했고, 실제 boundary 테스트는 policy 없는 app identity가 선택 bucket을 만들 때
기대한 `Created` 대신 typed `AccessDenied`를 반환한 assertion 차이로만 실패했다. 결과는 0 passed,
2 failed, 81 filtered out 및 `task_8_minio_boundary_red_exit=101`이다. RED card 자체는 policy,
identity, bucket 또는 object를 변경하지 않았다.

## Disposable MinIO least-privilege boundary GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just minio-boundary-green
printf 'task_8_minio_boundary_green_exit=%s\n' "$?"
```

이 card는 exact loopback/test guard 뒤 `jamye-task8-media` policy를 생성 또는 같은 이름으로
교체하고 기존 disposable app user에 부착한다. Policy에는 선택 bucket의 `CreateBucket`,
`GetBucketLocation`, `ListBucket`과 그 object의 `GetObject`, `PutObject`만 있다. App identity가
bucket 생성과 signed PUT/HEAD/GET을 전부 수행하며 admin credential은 policy 부착 및 테스트의
재현 가능한 전후 정리에만 사용된다.

반복 실행을 위해 실제 테스트는 고정된 `jamye-task8-media` bucket 안의 test object를 지우고
bucket을 삭제한 뒤 app identity로 다시 만든다. 이 삭제는 `JAMYE_ENVIRONMENT=test`, 정확한
`127.0.0.1:9000` health URL, 서로 다른 task-1 admin/app credential을 모두 만족할 때만 가능하다.
Sibling bucket, 다른 MinIO, production/homelab state, container/volume 및 Git state는 건드리지 않는다.

GREEN은 두 테스트가 모두 통과해 2 passed, 0 failed, 81 filtered out 및 exit `0`을 기록해야 한다.
Policy 적용이나 고정 test bucket 정리 뒤 테스트가 실패하면 같은 card를 재실행하면 된다. 테스트
시작 시 남은 고정 bucket을 다시 정리하므로 이전 중간 실패도 복구한다.

### 기록된 disposable MinIO boundary GREEN 증거

2026-08-27 사용자 실행에서 exact task-8 policy가 disposable app user에 부착됐고 production
crate와 두 MinIO boundary 테스트가 경고 없이 compile·발견됐다. App identity가 선택된 private
bucket을 직접 만들고 signed PUT/HEAD/GET을 수행했으며, sibling bucket 생성과 anonymous 접근은
차단됐다. Admin identity는 policy 부착과 loopback/test guard를 통과한 고정 bucket 전후 정리에만
사용됐다. 결과는 2 passed, 0 failed, 81 filtered out 및
`task_8_minio_boundary_green_exit=0`이다.

## Storage-only degradation and structured-log GREEN

```bash
just --justfile scripts/tasks/task-8/mod.just resilience-green
printf 'task_8_resilience_green_exit=%s\n' "$?"
```

이 card는 새 production 동작을 추가하는 RED/GREEN 단계가 아니라, 앞서 각각 유효한 RED/GREEN을
거친 readiness, C4 text-only write, MD1/MD2/MD4/MD5 stable storage error와 media HTTP logging
경계를 하나의 acceptance gate로 합성한다. 별도의 인위적인 RED는 만들지 않는다.

첫 테스트는 disposable PostgreSQL에 실제 message/event/outbox를 기록하면서 MinIO probe만
`degraded`, non-required로 보고되는지 확인한다. 같은 Router에서 body-only C4가 `201`을 반환하고
media 없이 정확히 한 message/event/outbox commit을 남겨야 한다. 나머지 두 테스트는 successful 및
failed presign/finalize/view/download 요청의 production JSON subscriber 출력을 줄마다 JSON으로
parse한다. Log에는 request ID와 stable event/error code만 있어야 하며 raw presigned URL, object key,
media/upload/target UUID, MinIO/S3 access·secret credential sentinel 및 AWS provider authorization
material이 없어야 한다.

두 log-capture 테스트는 thread-local subscriber를 설치하지만 `tracing` callsite interest cache는
process-global이므로 이 card는 세 acceptance 테스트를 `--test-threads=1`로 실행한다. 테스트 내부의
Tokio/PostgreSQL 동시성 검증은 그대로 유지하며, 병렬 libtest worker가 다른 subscriber context에서
동일 callsite를 먼저 등록해 캡처 결과를 바꾸는 test-runner 간섭만 제거한다.

이 검증은 실제 MinIO bucket/object를 변경하거나 container를 중지하지 않는다. PostgreSQL은
task-1 loopback/test guard 아래 무작위 disposable database 하나를 생성·migration한 뒤 삭제하고,
Git state와 Redis/MinIO state는 변경하지 않는다. GREEN은 3 passed, 0 failed, 83 filtered out 및
exit `0`을 기록해야 한다.

### 기록된 storage-only degradation/log GREEN 증거

2026-08-27 사용자 재실행에서 production crate가 경고 없이 compile됐고 세 acceptance 테스트가
모두 통과했다. MinIO만 non-required `degraded`인 상태에서도 body-only C4가 `201`을 반환해
message/event/outbox를 각각 한 번 커밋했고, 성공·실패 media HTTP log는 structured JSON을 유지하면서
presigned URL, object key, media/upload/target UUID, credential 및 provider authorization material을
노출하지 않았다. 공유 messaging test helper의 미사용 항목은 `tests/media.rs`의 해당 include module에만
사유가 있는 `dead_code` 허용을 적용했으며 재실행 출력에는 경고가 없었다. 결과는 3 passed,
0 failed, 83 filtered out 및 `task_8_resilience_green_exit=0`이다.

같은 날 repository-wide `cargo fmt --all -- --check`와
`cargo clippy --locked --all-targets --all-features -- --deny warnings`도 각각 exit `0`을 기록했다.

## Task-8 aggregate GREEN

모든 TDD slice와 resilience/log acceptance가 통과한 뒤 다음 카드로 Task-8 전체 media target과 기존
clean-architecture 경계를 한 번에 재검증한다.

```bash
just --justfile scripts/tasks/task-8/mod.just green
printf 'task_8_green_exit=%s\n' "$?"
```

카드는 task-1의 exact loopback/test 환경을 읽고 disposable MinIO app identity에 동일한
`jamye-task8-media` policy를 재부착한다. 이후 `dev-fixtures`를 포함한 media target에서 acceptance
세 테스트를 제외한 83개를 기존처럼 병렬 실행하고, `media_resilience_` 세 테스트는 새 Cargo
프로세스에서 직렬 실행한다. 마지막으로 all-feature architecture target을 실행한다. 프로세스 분리는
thread-local capture subscriber와 process-global `tracing` callsite cache 사이의 test-runner 간섭을
차단할 뿐 production 경로나 테스트 inventory를 생략하지 않는다. 현재 성공 기준은 합계 media
86 passed, 0 failed와 architecture 4 passed, 0 failed 및 최종 exit `0`이다.

Policy 재부착과 MinIO boundary test는 선택된 disposable `jamye-task8-media` bucket만 정리·재생성할 수
있다. PostgreSQL test는 무작위 disposable database만 생성·삭제한다. 실패하면 첫 diagnostic을 보존하고
수정한 뒤 이 카드 전체를 다시 실행한다. Persistent/production database, sibling bucket, container,
volume, Git state 또는 remote state는 변경하지 않는다.

### 기록된 Task-8 aggregate GREEN 증거

2026-08-27 사용자 실행에서 `cargo fmt --all -- --check`는 diff 없이 끝났고 repository-wide
`cargo clippy --locked --all-targets --all-features -- --deny warnings`도 경고 없이 통과했다. Aggregate
card의 첫 media 프로세스는 ordinary 83개를 모두 통과시키고 acceptance 세 개를 filter했으며, 새 직렬
프로세스는 `media_resilience_` 세 개를 모두 통과시켰다. 이어 architecture 네 테스트가 모두
통과했다. 합계 결과는 media 86 passed, 0 failed, architecture 4 passed, 0 failed 및
`task_8_green_exit=0`이다.
