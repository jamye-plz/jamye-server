# ADR 0009: 발신 단말 생성 영상 포스터와 `media_uploads` 연결

- 상태: Accepted
- 날짜: 2026-09-15
- 범위: 채팅 영상 첨부의 미리보기(포스터) 생성·저장·노출

## 맥락

수신 단말은 채팅 영상 미리보기 하나를 위해 원본 MP4 전체를 내려받아 로컬에서 t=0 프레임을
추출했다. 재생 시 같은 파일을 다시 받아 대역폭과 배터리를 이중으로 쓰고, 가시성이 짧게
꺼지면 진행 중이던 다운로드를 취소·재시작해 신뢰성이 낮았다. 서버 계약(`contracts/openapi.json`)에는
포스터·썸네일 개념이 없었고, 이 문제를 서버에서 해결하려면 다음 대안을 검토했다.

- **서버 측 ffmpeg 추출**: finalize 시점에 서버가 영상을 디코드해 포스터를 생성한다.
  프로세스·런타임 의존성이 추가되고, 수신 단말이 여전히 원본 영상을 다시 받아야 하는
  근본 문제(중복 다운로드)는 해결하지 못해 기각했다.
- **포스터를 메시지 첨부(`message_media`)로 노출**: 영상과 포스터를 각각 첨부 목록에 노출하면
  "메시지당 정확히 하나의 finalized media" 전제와 첨부 목록 UI 계약이 깨진다. 포스터는
  첨부가 아니라 영상의 메타데이터로 취급해야 하므로 기각했다.

## 결정

- 발신 단말이 첨부 시점에 로컬 영상에서 JPEG 포스터를 생성해 **별도 업로드**로 올린다.
  영상 업로드의 finalize 요청에 `poster_upload_id`를 포함하면, 서버는 같은
  user·scope·target으로 이미 confirmed된 image 업로드를 영상과 연결한다
  (migration `0010_media_posters.sql`, forward-only, [ADR 0003](0003-forward-only-sqlx-migrations.md)
  정책 준수: `media_uploads.poster_upload_id UUID NULL REFERENCES media_uploads(id)` +
  self-reference CHECK + partial unique index).
- 포스터 row는 첨부(`message_media`)를 만들지 않는다. 기존 업로드 상태 기계를 그대로 따라
  업로드·finalize 후 `confirmed`가 되고, 연결된 영상이 메시지에 바인딩되는 같은 transaction에서
  포스터도 `bound`(`bound_message_id` = 같은 메시지, `consumed_at` 기록)로 전이한다. 영상 바인딩이
  없거나 포스터가 만료/누락이면 영상만 바인딩하고 `poster_upload_id`는 NULL로 정리한다.
- 읽기 투영: `MessageAttachment`(REST 히스토리 `transport/http/chatrooms`,
  `adapters/postgres/chatrooms/query.rs`, realtime `contracts/realtime`)에
  `poster_media_id: uuid | null`을 추가한다. 접근 권한(`adapters/postgres/media/access.rs`)은
  첨부 row 없이도 `media_uploads.id`가 `status='bound'`이고 `bound_message_id`로 멤버십을
  통과하는 포스터 전용 경로를 별도로 검증한다.
- 제약: 포스터는 `image/jpeg`만, 크기 ≤ 1 MiB, chat scope의 영상 업로드에만 허용, 이미 다른
  영상에 연결된 포스터·자기 자신을 포스터로 갖는 영상은 거부. 위반은 기존 422 계열 오류 형식
  (`FinalizePolicyError` 확장)으로 응답한다.
- 서버는 MIME 타입과 바이트 크기만 강제한다. `media_uploads`에는 width/height 컬럼이 없어
  서버가 해상도를 검증할 수 없으며, 640px 캡은 발신 단말의 포스터 생성기가 보장한다
  ([jamye-app ADR 0006](../../../jamye-app/docs/adr/0006-media-video-posters.md)).

## 결과

- `migrations/0010_media_posters.sql` (forward-only, 기존 마이그레이션 미수정)로 스키마를
  확장했다. `ConfirmedUpload`에 `poster_upload_id: uuid | null`을 노출한다.
- OpenAPI/realtime 계약을 `just contract-generate`로 재생성하고 `just contract-check`를
  통과했다. `docs/`의 `media_uploads`/`finalize`/`MessageAttachment` 관련 서술
  (`docs/architecture.md` §7, `docs/roadmap.md`, `docs/migration-from-jamye-plz.md`)은
  포스터 이전과 동일하게 필드를 나열하지 않는 수준의 일반 서술이므로 갱신하지 않았다.
- 테스트: `tests/media`(finalize_policy/orchestration/adapters, message_policy/binding,
  access_*), `tests/contract.rs`, `tests/production_composition/migration_chain.rs`.
- 클라이언트 측 대응(발신 포스터 생성·업로드, 수신 포스터 우선 표시, 구 메시지 fallback)은
  jamye-app [ADR 0006](../../../jamye-app/docs/adr/0006-media-video-posters.md)에 기록한다.
