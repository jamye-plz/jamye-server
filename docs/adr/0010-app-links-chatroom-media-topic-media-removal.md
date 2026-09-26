# ADR 0010: 앱 링크 공개 route, 대화방 미디어 갤러리, 주제 미디어 제거

- 상태: Accepted
- 날짜: 2026-09-26
- 범위: 앱 M14 라운드 1 서버 지원(task-17): public app-link association, 초대 랜딩, 대화방 미디어 목록 API, `topic_media` 제거

## 맥락

앱 M14 라운드 1은 초대 링크를 OS app link/universal link로 열 수 있어야 하고, 주제 상세에서 "주제에 직접 붙은 미디어"가 아니라 해당 주제 대화방에 올라온 이미지·동영상을 갤러리로 보여야 한다. 기존 서버에는 topic-scope media upload/finalize와 `GET /api/v1/topics/{topic_id}/media`가 있었지만, 제품 결정 D5는 "주제 이미지" 개념을 서버까지 제거하라고 고정했다.

동시에 Android 개발 app link는 React Native template debug keystore 지문을 기본값으로 써야 한다. 이 키는 공개된 개발 키이므로 같은 package name으로 서명한 다른 앱도 검증을 통과할 수 있다. 이번 라운드에서는 개발 단계 수용 위험으로 기록하고, release/Play signing 값은 M18에서 교체한다.

## 결정

- API origin이 인증 없는 세 route를 제공한다.
  - `GET /.well-known/apple-app-site-association`
  - `GET /.well-known/assetlinks.json`
  - `GET /invite/{code}`
- Association route는 JSON만 반환하고 redirect하지 않는다. 초대 랜딩은 코드 형식(`[A-Za-z0-9_-]{16,64}`)만 검사한다. 형식이 맞더라도 DB를 조회하지 않으므로 초대 유효성, 그룹 이름, membership 상태를 노출하지 않는다.
- 새 설정은 `JAMYE_APP_LINKS_AASA_APP_IDS`, `JAMYE_APP_LINKS_ANDROID_PACKAGE`, `JAMYE_APP_LINKS_ANDROID_SHA256_CERT_FINGERPRINTS`, `JAMYE_APP_STORE_URL`, `JAMYE_PLAY_STORE_URL`이다. 스토어 URL은 설정 시 `https`만 허용한다.
- 대화방 갤러리는 `GET /api/v1/chatrooms/{chatroom_id}/media`(`C5`)로 제공한다. 권한은 기존 chatroom membership 경로를 재사용하고, 없는 방과 비멤버는 모두 403 `membership_required`다. item cursor는 `message_media.id`, 정렬은 `messages.created_at DESC, messages.id DESC, message_media.position ASC`다. 이미지·동영상만 반환하고 음성은 제외한다.
- 주제 미디어는 서버에서 제거한다. `MediaScope::Topic`, topic upload/finalize, `topic_media`, `CanonicalTopic.media`, MD3 계약을 삭제하고, `migrations/0012_remove_topic_media.sql`가 forward-only로 스키마를 chat-only로 바꾼다.
- `account_object_deletion_intents`는 임의 object key를 저장할 수 있으므로, topic media와 topic-scope upload object key는 기존 object cleanup worker 경로로 삭제 예약한다. 고아 object로 남기는 대안은 기각했다.

## 대안

- Association을 별도 정적 호스트나 CDN에서 제공: API 배포와 association 값이 갈라지고 homelab 변경이 필요해 이번 task 범위를 키운다. 기본값 배포를 위해 API origin route를 선택했다.
- 초대 랜딩에서 초대 유효성이나 그룹 이름을 조회: 사용자에게 더 친절하지만 enumeration oracle이 된다. 랜딩은 정적 안내와 code copy/open-app affordance만 제공한다.
- 주제 상세 갤러리를 topic-owned media로 유지: D5와 충돌하고 앱이 요구한 "주제 대화방에 올라온 이미지·동영상" 의미와 다르다. 대화방 attachment projection을 선택했다.
- Topic object를 고아로 남김: `account_object_deletion_intents`가 `object_key` 중심 cleanup intent를 이미 제공하므로 기각했다.

## 결과

- `contracts/openapi.json`의 selected REST index는 `MD3`를 제거하고 `C5`를 추가한 43개 operation이다.
- `media_uploads.scope`는 `chat`만 허용하고 `media_uploads_consumer_shape_check`는 chat 전용 pending/confirmed/bound shape을 유지한다.
- `topic enriched` 상태는 PATCH body 경로만 남는다. topic media 업로드나 목록 조회는 없다.
- 배포 전 운영 DB에서는 다음 읽기 전용 count SQL로 삭제 대상과 삭제 예약 대상 object 수를 기록한다.

```sql
SELECT 'topic_media_rows' AS metric, count(*)::bigint AS value
FROM topic_media
UNION ALL
SELECT 'topic_scope_upload_rows' AS metric, count(*)::bigint AS value
FROM media_uploads
WHERE scope = 'topic'
UNION ALL
SELECT 'distinct_topic_object_keys' AS metric, count(DISTINCT object_key)::bigint AS value
FROM (
    SELECT object_key FROM topic_media
    UNION ALL
    SELECT object_key FROM media_uploads WHERE scope = 'topic'
) AS topic_objects;
```

## 검증

- S1: public association/invite route HTTP 테스트와 CSP hash 테스트.
- S2: C5 HTTP 테스트(401, 403, 이미지·동영상 필터, 최신순, 메시지 경계 pagination, cursor/limit 422, poster 포함).
- S3: 0012 migration 테스트(topic table/column 제거, topic scope 거부, chat consumer CHECK 유지), contract contribution 테스트, production migration chain 12개 확인.
