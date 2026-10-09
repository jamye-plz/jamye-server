# ADR 0013: UGC 신고·차단·정지와 메시지 필터

## Status

task-20b 서버 구현에서 채택. 아직 운영에 배포하지 않았다.

## Context

스토어 심사(Apple 1.2, Play UGC)는 사용자 생성 콘텐츠 앱에 신고, 차단, 부적절한 콘텐츠 필터링,
운영자의 신속한 조치를 요구한다. 서버는 이미 메시지·주제 본문, 푸시 파이프라인, 계정 삭제 purge,
세션 폐지 경로를 갖고 있다. 이 ADR은 그 위에 얹는 최소 데이터 모델과 결정을 기록한다.
약관 동의는 클라이언트에만 두며 서버에 endpoint나 table을 만들지 않는다(가정 A12).

## Decision

- **migration `0020`은 forward-only다.** `user_blocks`(PK `(blocker_id, blocked_id)`, 자기 차단 CHECK,
  `blocked_id` index), `reports`(대상은 메시지 또는 사용자 정확히 하나, reason·status·handled 상태
  CHECK, 메시지 snapshot jsonb, `(status, created_at, id)` index), `users.suspended_at`·
  `suspension_reason`(nullable)을 더한다. 약관 table과 감사(audit) table은 만들지 않는다. 조치 기록은
  구조화 로그 한 줄이다(가정 A13).
- **차단은 단방향이며 전달을 바꾸지 않는다.** `PUT/DELETE /api/v1/me/blocks/{user_id}`는 멱등이고
  멤버십·메시지 전달을 건드리지 않는다. 차단한 사용자는 메시지 알림(notification 행과 푸시)만 받지 않는다.
  새 주제(`new_topic`) 알림은 바뀌지 않는다. 숨김은 클라이언트가 맡는다.
- **신고는 항상 201이다.** 대상이 없거나 접근할 수 없으면 구분 없이 같은 404
  `report_target_not_found`, 자기 신고는 422 `report_self_target`, 중복은 허용한다. 자유 텍스트는 받지
  않는다. 메시지 신고는 신고 시점의 본문(마스킹된 텍스트, 최대 4000자), content type, 미디어 참조를
  snapshot으로 남겨 작성자가 삭제해도 운영자가 조치할 수 있게 한다. 신고 제한은 사용자당 시간당 20회
  고정값이다(가정 A23, 새 환경 변수 쌍 없음). 초과 시 429 `rate_limit_exceeded`와 `Retry-After`로 다른 endpoint와 같은 코드를 쓴다.
- **정지는 모든 인증 진입점에서 403 `account_suspended`다.** bearer extractor는 검증된 토큰 뒤에
  account gate(`users.suspended_at` 조회)를 기다리고, refresh와 OAuth/Apple 교환은 저장소 단계에서,
  realtime ticket 발급(extractor)과 WebSocket 연결(ticket 소비 직후)은 같은 gate로 거부한다.
  정지는 refresh session을 폐지(`revoked_at`)하고, 해제는 컬럼만 지운다. 폐지된 session은 되살리지 않으므로
  사용자는 다시 로그인한다. 이미 열린 WebSocket은 강제로 끊지 않는다(다음 재연결에서 거부).
  extractor는 요청마다 PK 조회 한 번을 더한다. gate 실패(DB 불가)는 fail-closed 503이다.
- **운영자 알림은 기존 푸시 파이프라인을 쓴다.** 신고가 commit된 뒤 `JAMYE_OPERATOR_ACCOUNT_IDS`의 계정이
  가진 활성 설치마다 `push_delivery_intents` 행 하나를 만든다(`report_id`로 신고에 묶고
  `(report_id, push_installation_id)`는 유일). 알림 행은 notification·conversation event가 없으므로
  migration이 두 NOT NULL을 풀고 CHECK로 "대화 occurrence" 또는 "신고 알림" 둘 중 하나의 모양만
  허용한다. 기존 worker가 claim하고 `authorize_claim`이 occurrence 종류(`report_id` 유무)에 맞는
  경로 하나만 골라(신고 알림은 설치·계정 재검증, 대화 occurrence는 기존 인가) 검증한 뒤 Expo로
  보낸다(성공·재시도·DeviceNotRegistered 처리는 대화 푸시와 같다). 제목·본문은 고정 한국어 문구이고
  data는 `{type: "report", report_id}`뿐이다. enqueue 실패는 로그만 남기고 신고를 실패시키지 않는다.
  설정이 없으면 보내지 않고, 형식이 틀린 id 또는 10개 초과는 시작을 실패시킨다.
- **R8 필터는 마스킹이다(거부하지 않는다).** 서버가 채팅 본문(주제 안내 메시지 포함)을 받는 곳에서,
  저장·이벤트·outbox·푸시 미리보기·sync·신고 snapshot보다 앞서 목록의 용어를 대소문자 무관하게 `***`로
  바꾼다. 목록에 없는 본문은 바이트 단위로 같다. 목록은 `src/domain/moderation/default_terms.txt`에
  내장된 짧은 심각 비하·혐오 용어이고, `JAMYE_CONTENT_FILTER_TERMS_FILE`(한 줄에 용어 하나)로 통째로
  바꿀 수 있다. 읽을 수 없거나 비어 있으면 시작이 실패한다. 닉네임·그룹 이름·주제 제목 같은 다른 텍스트는
  거르지 않는다(가정 A26). 부분 문자열 매칭이므로 무해한 단어가 가려질 수 있다.
- **purge.** 계정 purge는 그 계정의 차단을 양방향으로 지우고, 신고는 보존하되 신고자·대상 사용자 참조를
  기존 익명 tombstone 사용자로 옮긴다. 그 계정이 운영자였다면 신고 알림 occurrence를 먼저 지운다.

## Consequences

- 장점: 새 table 두 개와 컬럼 두 개로 신고·차단·정지가 닫히고, 푸시 전달·재시도·무효 토큰 처리를 다시
  만들지 않는다.
- 비용: `push_delivery_intents`가 두 모양을 갖는다(CHECK로 보호). 모든 인증 요청에 정지 조회가 붙는다.
  정지 시 열린 WebSocket이 즉시 끊기지 않는다.
- 후속: 서버측 약관 동의 기록, 신고 보존 기간 자동 정리, 정지 시 realtime 연결 강제 종료, 용어 목록의
  부분 문자열 오탐 완화(단어 경계)는 별도 승인 항목이다.
