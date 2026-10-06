# ADR 0012: 서버 호스팅 프로필 사진(아바타)

## Status

task-19 서버 구현에서 채택. 아직 운영에 배포하지 않았다.

## Context

앱은 프로필 사진을 직접 올리고 지울 수 있어야 한다. 앱의 아바타 표시 코드(iOS
`JamyeAvatarView`, Android Compose `Image`)는 인증 헤더 없이 `avatar_url`을 읽는다. 기존
미디어 경로는 쓸 수 없다. 업로드는 대화방 대상이 필요하고, 읽기는 그룹 멤버십과 만료 600초의
presigned URL을 거치며, bucket은 비공개다. 그리고 `avatar_url`은 512자 이하 https 문자열이라
서명된 URL이 맞지 않는다.

## Decision

- **공개하지만 추측할 수 없는 고정 URL.** 확정된 아바타는
  `{JAMYE_AVATAR_PUBLIC_BASE_URL}/api/v1/avatars/{upload_id}`로 읽는다. `upload_id`는 서버가
  업로드마다 새로 만드는 UUIDv4이므로 URL은 바뀌지 않고(immutable 캐시) 사용자 id를 드러내지 않는다.
  카카오·Google 프로필 사진과 같은 공개 URL 방식이라 앱의 표시 코드는 그대로다.
- **redirect가 아니라 스트리밍.** API가 비공개 bucket에서 객체(최대 1 MiB)를 읽어 응답한다.
  redirect는 만료되는 서명 URL을 `avatar_url`에 고정할 수 없고, bucket을 공개하면 MinIO 정책과
  노출 범위를 넓혀야 한다. 스트리밍은 bucket을 비공개로 두고 교체·지움·purge 뒤 즉시 404를 보장한다.
  응답 헤더는 `Content-Type`, `Cache-Control: public, max-age=31536000, immutable`,
  `X-Content-Type-Options: nosniff`뿐이다. URL이 불변이므로 ETag/304는 두지 않는다.
  없음·비UUID·비active는 모두 `Cache-Control: no-store` 404 `avatar_not_found`다.
- **상태 모델.** `user_avatar_uploads`(migration 0019)의 행은 `pending`(U4 시작) →
  `active`(U5 확정) → `released`(교체·지움·새 intent로 대체)로만 움직인다. 사용자당
  pending·active는 각각 부분 unique index로 최대 1개다. 같은 `upload_id` 재확정은 멱등이다.
  U5는 `users` 행을 잠근 한 transaction 안에서 이전 active를 release하고 새 행을 active로
  만들며 `users.avatar_url`을 갱신하므로 동시 호출이 서로를 앞지르지 못한다.
- **삭제 큐 재사용.** 객체를 지울 일은 모두 기존 `account_object_deletion_intents`(object_key
  unique, at-least-once)에 넣고 기존 cleanup worker가 지운다. 새 worker와 테이블은 없다.
  교체(U5), 지움·다른 https 값(U2), 대체된 pending(U4)은 release 시점에 큐에 넣고,
  계정 purge는 남은 pending·active 객체를 큐에 넣은 뒤 모든 행을 hard-delete한다.
- **설정 게이트.** `JAMYE_AVATAR_PUBLIC_BASE_URL`(https origin, 256자 이하)이 없으면 U4~U6
  route를 mount하지 않는다. 코드를 꺼진 채 먼저 배포하고, 인증 없는 smoke(꺼짐 404 / 켜짐 401)로
  상태를 구분한 뒤 homelab에서 값을 연결한다. 값은 기존 origin 검증기와 `validate_url`로 검증하고
  잘못되면 시작에 실패한다. U4는 기존 media presign limiter 값을 재사용(counter key 분리)하고,
  U5는 새 limit이 없으며, U6만 `JAMYE_RATE_LIMIT_AVATAR_PUBLIC_READ_*`(기본 600/60초)를 둔다.
- **U2와의 경계.** U2는 계속 임의 https 문자열을 받는다. 호스팅 소유 관계는 U5에서만 생기고,
  U2가 `""` 또는 다른 값을 보내면 active 호스팅 행을 해제한다. 타인의 호스팅 URL을 U2로
  넣어도 본인 화면만 깨지므로 별도 규칙을 만들지 않는다.

## Consequences

- 객체는 `image/jpeg`, 1 MiB 이하만 허용한다. 서버는 크기·content type·시그니처(`FF D8 FF`)만
  확인하고 디코딩하지 않으며, EXIF 제거는 앱의 512px JPEG 재인코딩이 맡는다.
- U6은 요청마다 객체를 메모리(최대 1 MiB)로 읽는다. 공개 읽기 한도는 ConnectInfo(peer 주소)
  기준이라 reverse proxy 뒤에서는 proxy 주소 단위 bucket이며 기본값은 사실상 전역 상한이다.
  `X-Forwarded-For` 처리는 MVP에서 하지 않는다.
- 새 객체 prefix `avatar/`가 운영 MinIO 자격 증명 정책에서 PUT/GET/DELETE 가능해야 한다.
  배포 전 안전 리뷰가 정책 파일을 확인하고, 실제 확인은 첫 업로드로 한다.
- 계정 삭제 유예 중에는 active 행이 계속 서빙되고 purge가 행을 지우면 같은 URL이 404가 된다.
- 한 사용자는 동시에 하나의 아바타만 가진다. 이전 사진은 교체 즉시 URL이 404가 되고
  객체는 cleanup worker가 지연 삭제한다.
