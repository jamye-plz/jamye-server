# jamye-server 로드맵 — FastAPI 전체 이관, 신뢰성 고도화, 모바일 계약

> 세션: ultrawork/20260822-200110 · 후속 로드맵 등록: ultrawork/20260922-100557 · task-14 구현: ultrawork/20260928-171401
> 현재 단계: task-14 1·2·3차 운영 배포 완료(soft delete 전 범위, 계정 삭제 유예·복구, 앱 M15 소프트 삭제 수용 연동). task-15·16은 여전히 착수 승인 대기(task-14를 선행으로 둠, §14)
> 상태: Task 13이 완료됐다(근거는 §9). task-14(soft delete)는 세션 ultrawork/20260928-171401에서 구현해 1차(merge `c047e5a`)·2차(merge `b20a4d0`)·3차(merge `6dcb6d5`)를 모두 운영 배포했다(§0, §13, §14). task-15(Apple exchange)·task-16(서버측 잔여 백로그)는 2026-09-22 로드맵에 등록만 됐으며 착수는 별도 승인이 필요하다(task-14 완료가 둘의 선행 조건). task-17은 2026-09-26 앱 M14 라운드 1 지원 범위로 구현해 같은 날 운영 배포했고(merge `5b987a2`, migration `0012`), task-18은 2026-09-27 앱 M14 라운드 2 지원 범위로 구현해 같은 날 운영 배포했다(merge `a77cac5`, migration `0013`). 2026-09-28에는 HTTP 메시지 전송의 첨부 1개 제한을 푼 수정(merge `c7f71a8`)을 배포했다(§13).
> 진행률: Task 1-14·17-18 구현·배포 완료, task-15-16은 등록(planned_unapproved) 단계
> 기계 SSOT: .agents/results/plan-20260822-200110.json (task-1-13) · .agents/results/plan-20260926-181036.json (task-17) · .agents/results/plan-20260927-120934.json (task-18) · .agents/results/plan-20260928-171401.json (task-14) · task-15 이후는 착수 시 새 plan JSON 생성

## 0. 2026-09-28~29 task-14 서버 구현·배포 기록

task-14는 세션 `20260928-171401`에서 1·2·3차로 나눠 구현하고 모두 운영 배포했다(배포 절차·날짜·merge commit·PR·migration 번호는 §13, 상세 범위는 §14 task-14). 1차 범위는 계정 삭제를 즉시 D10 hard delete로 처리하지 않고 `users.deleted_at` 기반 30일 유예로 시작하며, refresh session 폐기, push installation 비활성화, membership soft delete(`account_deleted_at`)와 기존 realtime control eviction 경로를 결합한다. retained author link(`sender_id`/`author_id`)는 유예 동안 보존하고, C2/C4/S1/T3/T4/G4/N1 조회 투영에서만 `탈퇴한 사용자`/`null`로 익명화한다.

A2 OAuth exchange는 유예 중 soft-deleted identity를 별도 복구 경로에서만 되살리고, 복구 시 `X-Jamye-Account-Restored: true` 응답 헤더를 추가한다. `TokenPair` 본문은 바꾸지 않는다. 30일 이후 purge worker는 DB clock 기준 lease/batch claim으로 기존 D10 전이를 실행한다. 계약 v2 협상과 삭제 이벤트는 1차 base/delete 단계에서 추가됐고, 1차 grace 단계가 복구 헤더와 soft-delete schema를 더했다. 2차는 나머지 B 대상(초대·푸시·알림·읽음 위치·태그·업로드 등)을 live-row partial unique로 전환했고, 3차는 기기 검증에서 찾은 결함을 고쳐 `topic.deleted` 이벤트를 삭제되는 주제 대화방이 아니라 그룹 메인 대화방 피드에 기록하도록 바꿨다(migration 없음).

## 1. 목표와 범위

이번 작업의 목표는 기존 PWA + FastAPI monorepo에서 서버를 분리해 Rust/Axum 기반 jamye-server로 재설계하고, 동시에 개발되는 React Native 앱이 사용할 계약을 C2 release candidate까지 제공하는 것이다.

현재 승인 범위는 다음 세 가지다.

1. /Users/poby/Developer/jamye-plz/backend에서 검증된 FastAPI capability를 보존 또는 승인된 변경으로 이관한다. 단, 사용자가 D3=C로 확정한 STT/전사는 non-goal이며 일반 voice media 송수신·재생은 보존한다.
2. 메시지 전달을 PostgreSQL authoritative state, durable outbox, Redis Pub/Sub, WebSocket acceleration, REST delta sync 구조로 고도화한다.
3. OpenAPI 3.1, realtime JSON Schema, fixture, manifest를 deterministic artifact로 만들고 모바일 앱에 전달할 준비를 마친다.

최초 프롬프트의 “첫 수직 절편만” 제한보다 사용자의 이후 “백엔드 전체 이관 + 고도화 + C2 계약” 지시가 우선한다. 그러나 production 데이터 import/cutover, production credential, 실제 배포, contract publication, jamye-app lock publication, commit/push/tag는 현재 범위가 아니다.

이 문서는 사람이 이해하기 위한 projection이다. 충돌하면 기계 계획이 우선한다.

## 2. 지금 적용되는 승인 경계

- 계획 단계는 종료됐고 사용자가 별도로 “M0 시작”을 승인했다. task-1 M0, `docs/migration-from-jamye-plz.md`의 task-2 M1 scope lock, task-3a core schema가 완료됐다. 사용자가 2026-08-25에 D1=A, D8=A, D12=A, D13=A를 명시적으로 선택했으며 각 earliest materializer가 해당 evidence를 소비한다.
- M0 다음 feature, migration, generated contract와 production composition은 해당 dependency와 decision gate가 열리기 전에는 구현하지 않는다.
- 루트 `Justfile`이 유일한 공개 명령 목록이다. task 번호, RED/GREEN card, manifest digest는 활성 검증에 사용하지 않는다.
- 별도 Bash는 credential/trap/wait/guarded deletion과 서비스 stop/start recovery처럼 실제 안전 경계를 제공할 때만 둔다.
- pending 제품 결정은 가장 이른 materializer가 한 번만 사용자 선택을 받아 evidence를 고정한다. 후속 task와 VERIFY/SHIP는 dependency를 통해 그 evidence를 소비하며 같은 결정을 다시 승인받지 않는다.
- production/release/SCM 변경은 별도 승인이 있어야 한다.
- legacy jamye-plz, homelab, 운영 PostgreSQL/Redis/MinIO는 읽기 전용 또는 범위 밖이다.
- task-14~16은 2026-09-22 로드맵 등록만 됐고 각 task 구현·migration·contract publication·배포는 별도 승인이 필요하다. task-17(2026-09-26)과 task-18(2026-09-27)은 각 세션에서 사용자가 배포를 승인해 운영 백업·읽기 전용 확인 SQL·배포 절차를 거쳐 배포됐다(§13).

## 3. 목표 아키텍처

~~~mermaid
flowchart LR
    A[React Native 앱] --> L[(SQLite 메시지 + local outbox)]
    L -->|같은 client_msg_id로 REST command| B[Axum API]
    B -->|한 PostgreSQL transaction| P[(PostgreSQL)]
    P --> M[messages]
    P --> E[conversation_events]
    P --> O[outbox_events]
    O --> W[Rust outbox worker]
    W -->|PUBLISH| R[(Redis Pub/Sub)]
    R --> N1[Axum API node 1]
    R --> N2[Axum API node 2]
    N1 -->|authorized local WS| A
    N2 -->|authorized local WS| A
    A -. 두 단계 paginated delta sync .-> B
    B -. authoritative events .-> P
~~~

핵심 의미는 다음과 같다.

- 앱의 SQLite 경계는 다음 문장을 계약에 그대로 싣는다: `One exclusive SQLite transaction creates both messages(status=pending) and the persisted outbox command with the unchanged client_msg_id; after process reopen, both rows are visible or neither is.` 실행 검증은 jamye-app 소유다.
- 서버 메시지 command는 인증된 REST다. body의 client_msg_id가 authoritative idempotency key이며 optional Idempotency-Key header는 같은 값이어야 한다.
- 서버는 messages, conversation_events, outbox_events를 한 transaction에서 기록한다.
- Redis와 WebSocket은 빠른 전달 경로일 뿐이다. Redis 유실은 PostgreSQL에 커밋된 메시지 유실이 아니다.
- 앱은 foreground, network regain, WebSocket reconnect, validated push tap에서 같은 delta-first sync engine을 실행한다.
- PostgreSQL이 사용자, 권한, 메시지, 이벤트, 알림, durable intent의 원본이다.

## 4. 계약 단계

| 단계 | 목적 | 포함 범위 | 완료 권한 |
|---|---|---|---|
| C0 | 모바일 채팅을 시작할 수 있는 최소 runtime-adjacent 계약 | 공통 wire/error, H1/H2, C4, S1, R1, WebSocket protocol/close, message.created, outbox/delta와 ordinary-401 보존 marker | task-3b |
| C1 | 실제 메시지 수직 경로 | REST message → PostgreSQL outbox → worker → Redis → authorized WS → paginated delta recovery | task-4a/task-4b |
| C2 | 선택된 전체 서버 계약 release candidate | 모든 runtime owner의 DTO/schema/fixture contribution, selected REST inventory, 최종 2 realtime variants, manifest provenance | task-12 |
| C3 | 후속 soft delete/Apple 계약 확장(2026-09-22 등록, planned_unapproved) | 메시지 삭제(가칭 C6)·주제 삭제(가칭 T8) REST, `deleted_at`/`updated_at` 필드, realtime `message.deleted`/`topic.deleted`, Apple exchange endpoint(가칭 A6) | task-14/task-15 |

계약 단계 C3는 operation ID `C3`(read marker; `contracts/openapi.json`)와 이름만 같고 무관하다. 메시지 삭제의 가칭은 처음 C5였지만 task-17이 `C5`를 대화방 미디어 목록에 쓰게 되어 C6으로 바꿨다. contract version 정책(현재 버전 1 유지 vs 버전 2로 증가, realtime event version 협상 포함)은 task-14 PLAN에서 결정한다.

C0는 정확히 5개 REST operation(H1, H2, C4, S1, R1)과 message.created 하나만 생성한다. D1, D8, D13만 C0를 막을 수 있다. 인증, 그룹, 주제, 미디어, 알림, 푸시, 계정 삭제 계약은 각 runtime feature owner가 나중에 추가한다.

후속 PWA→React Native 교체 지시로 D2는 Expo-only로 고정되었다. 선택된 최종 REST index는 task-17에서 `MD3` 주제 미디어 목록을 제거하고 `C5` 대화방 미디어 목록을 추가해 정확히 43행이며, Web Push/VAPID 관련 runtime/schema/table/config/Nix surface는 만들지 않는다.

C2 realtime union은 `message.created`, `topic.created` 두 가지다. STT field/event/job/worker/provider/package는 C2에 없다.

Voice message는 body 없이 정확히 하나의 finalized audio media를 가진 일반 message다. 실제 object HEAD의 MIME/size와 container duration을 검증한 뒤 같은 `client_msg_id`로 message+message_media+conversation_event+outbox를 한 transaction에 기록한다. history, delta/`message.created`, authorized short presigned GET 재발급으로 재생할 수 있으며 transcript field에 의존하지 않는다.

## 5. 모바일 outbox와 delta sync 경계

jamye-app이 소유하는 실행 로직:

- 위 canonical 문장대로 한 exclusive SQLite transaction에서 `messages(status=pending)`와 persisted outbox command를 같은 `client_msg_id`로 만들고, reopen 뒤 둘 다 보이거나 둘 다 보이지 않게 한다.
- restart 후에도 같은 outbox row와 같은 client_msg_id를 재사용한다.
- applied_events는 event_id를 UNIQUE로 적용한다.
- 각 page의 event apply transaction이 커밋된 뒤에만 conversation cursor를 monotonic CAS로 전진시킨다.
- network/timeout/429/5xx는 capped retry, 안정적인 403/404/409 idempotency conflict/422는 terminal 또는 명시적 사용자 조치, 426은 upgrade-required stop으로 처리한다.
- SecureStore, refresh single-flight, retry scheduler, foreground/network/reconnect/push trigger 실행 테스트는 앱 저장소가 소유한다.

jamye-server가 보장하고 handoff fixture로 명시하는 순서:

1. 마지막 커밋 cursor에서 delta phase 1을 시작하고 EventPage pagination을 null/empty까지 모두 비운다.
2. one-time realtime ticket을 발급한다.
3. WebSocket을 연결하고 subscribe acknowledgement를 받는다.
4. acknowledgement 뒤 delta phase 2도 null/empty까지 모두 비운다.
5. 각 page는 apply commit 뒤 cursor CAS 순서를 지킨다.
6. same/regressing next_cursor는 안전하게 중단한다.
7. bounded guard는 이미 커밋된 progress를 보존하고 다음 실행에서 재개한다.

검증 fixture에는 2×limit보다 큰 backlog, terminal empty page, page 사이 새 commit, join-gap event A/B, WS/page duplicate와 out-of-order, UnsupportedEventMarker가 page를 넘는 경우가 포함된다. executable pagination loop는 앱 소유다.

## 6. 한 가지 모바일 인증 오류 분류

- 첫 ordinary 401: 영향을 받은 outbox command를 보존·일시정지하고 한 개 A3 refresh single-flight에 합류한다.
- refresh 성공: SecureStore를 먼저 교체한 뒤 원래 client_msg_id로 각 command를 한 번만 replay한다.
- invalid/reused refresh 또는 replay의 두 번째 401: reauthentication, loop 금지.
- 안정적인 403/404/409 idempotency conflict/422: terminal 또는 명시적 사용자 수정.
- network/timeout/429/5xx: capped retry.
- 426: upgrade-required stop.

C0는 ordinary 401에서 outbox intent와 client_msg_id를 보존하고 후속 auth 계약으로 넘긴다는 marker만 전달한다. 정확한 A3 DTO, refresh-family fence, SecureStore 교체 순서, 전체 오류 분류와 static two-send/one-refresh trace의 유일한 publisher는 task-5다. task-12는 이를 C2 fixture에 결합하고, 최종 감사와 실행 테스트는 각각 VERIFY/SHIP와 jamye-app이 소유한다.

## 7. 신뢰성 규칙

### PostgreSQL outbox

- 메시지 transaction은 messages + conversation_events + outbox_events를 원자적으로 기록한다.
- UNIQUE(sender_id, client_msg_id)가 DB 수준 멱등성 경계다.
- worker는 claim/reclaim마다 generation을 증가시키고 completion은 id, owner, captured generation, live lease를 모두 비교한다.
- PostgreSQL clock_timestamp()가 claim, lease, renewal, deadline, completion의 authoritative time이다.
- Redis publish timeout + safety margin은 lease보다 짧다.
- crash-after-publish duplicate는 허용하지만 stale claimant가 durable state를 바꾸는 일은 허용하지 않는다.

### Durable worker 공통 규칙

- Redis publish, Expo call, object HEAD/delete 같은 짧은 I/O는 검증된 timeout과 safety margin을 가지며 합이 lease보다 짧다.
- duplicate external work는 가능하지만 stale durable mutation은 불가능해야 한다.
- timeout/reclaim/stale-completion 테스트는 deterministic barrier/clock control로 작성한다.
- 각 worker owner가 timeout/lease 값을 feature-local non-secret config로 정의·검증하고, Task-13의 Nix 배포 단계가 최종 `.env.example`과 NixOS module에 그 값을 노출한다.

### Push send authorization linearization

한 DB-time transaction이 다음 순서로 row를 lock한다.

live group → live membership → recipient notification → installation → delivery occurrence

이 transaction은 group 삭제 여부, 현재 membership, notification owner, installation owner/epoch/enabled/current-preview, occurrence generation/live lease를 검사하고 authorization을 커밋한 뒤 DB connection을 놓는다. message preview text는 이 커밋 뒤에만 canonical message에서 파생한다.

membership revoke, group delete, P2 rebind, P3 preview/disable, P4 delete, U3 account deletion도 같은 lock order/fence를 사용한다. P2는 `platform=ios|android`, environment, 전역으로 유일한 installation_id, Expo token을 요구하고 provider는 서버가 `expo`로 고정한다. P3/P4는 그 전역 installation_id 하나를 정확히 가리킨다.

- privacy/membership mutation이 먼저 commit되면 provider call과 message-derived text는 0회다.
- authorization이 먼저 commit되면 이미 승인된 in-flight attempt 하나까지만 끝날 수 있다.
- 이후 mutation은 모든 retry와 later attempt를 막는다.

claim 전후와 provider-start interleaving을 모두 테스트한다. U3가 먼저 commit되면 call/text/retry가 0이고 reclaim 가능한 occurrence도 남지 않는다. authorization이 먼저면 이미 승인된 시도 하나까지만 끝날 수 있으며 늦은 result CAS는 durable row를 바꾸지 못한다.

### 정적 cross-feature transaction orchestration

runtime contribution registry, plugin hook, 동적 feature 등록과 feature별 병렬 UnitOfWork interface는 두지 않는다. task-4a/backend가 유일한 최소 opaque `TransactionHandle`/manager port와 SQLx 구현을 소유한다. application wrapper만 handle을 시작하고 한 번 commit하며 repository는 시작하거나 commit하지 않는다. 후속 feature의 composable PostgreSQL operation은 모두 같은 caller-owned handle을 인자로 받으므로 application/domain은 SQLx를 import하지 않는다. feature-local standalone wrapper가 필요해도 이 handle을 그대로 열고 닫을 뿐 새 transaction abstraction을 만들지 않는다.

task-12/backend는 transaction port/adapter를 다시 만들지 않고 task-4a의 frozen handle을 소비해 다음 세 UoW의 최종 호출 순서만 정적 코드로 완성한다.

Task-12 production composition은 task-5의 validated `AuthConfig` secret을 Kakao/Google
provider와 access-token codec에 실제로 연결한다. 이 연결 전 반복되는 두 `dead_code` 경고는
`src/config/auth/mod.rs`의 좁은 `#[expect(dead_code)]`로만 억제하며, task-12는 실제 사용 연결과
동시에 두 expectation을 제거해 경고가 억제 없이 해소됐음을 검증한다.

- `SendMessage`: message/event/outbox → media binding(voice는 bodyless exactly-one finalized audio) → unread notification/push occurrence
- `CreateTopic`: topic/chatroom/bootstrap/announcement/read/event/outbox → notification/push occurrence
- `MarkConversationRead`: monotonic read marker → bounded notification clear

각 단계 뒤 failure injection은 누적 write set 전체 rollback을 증명한다. task-6 그룹 mutation+task-6c control intent, task-11 account deletion+push privacy fence+object-delete intent도 task-4a의 같은 handle과 caller-owned one-commit 규칙을 따른다. outbox dispatch는 closed enum의 static match다. task-12는 이 세 실제 PostgreSQL UoW와 최종 api/worker reachability를 한 integration target으로 구현하고, feature별 의미 테스트는 원래 owner가 유지한다.

## 8. Migration과 contract provenance

### SQLx migration

- 한 개 ADR이 forward-only SQLx migration 정책을 정의한다.
- speculative down migration 파일은 만들지 않는다.
- 각 numbered migration은 reversibility/forward-fix rationale metadata와 recovery reference를 포함한다.
- 각 migration owner는 실제 schema prerequisite를 가진 disposable DB에서 transactional up/upgrade와 forced-failure rollback을 증명한다.
- `0001`은 `chatrooms.topic_id`를 nullable UUID, CHECK, partial index로만 만들고 FK는 만들지 않는다. `0005`가 `topics`를 만든 뒤 `chatrooms.topic_id REFERENCES topics(id)`를 추가한다.
- `0007`은 notifications/push, `0008`은 account deletion/object cleanup을 소유한다. 각 owner는 바로 앞 numbered schema를 가진 disposable DB에서 검증하며, task-12와 VERIFY가 canonical `0001→0008` fresh chain과 upgrade를 검증한다. persistent/production DB에는 partial chain을 적용하지 않는다.
- disposable local reset은 guarded `just infra-reset` 명령으로 문서화한다.
- production restore/import/cutover는 별도 승인 대상이다.

### Frozen legacy evidence

M1은 /Users/poby/Developer/jamye-plz의 정확한 PF1 source set을 deterministic sort로 고정한다.

- 문서 7개: server initial prompt의 authoritative span 8-304, app initial prompt의 authoritative span 1-319, vision/scope, features, API contract, data model, NixOS deployment 문서
- backend/app/**/*.py
- backend/alembic/versions/*.py
- backend/tests/**/*.py
- cache/bytecode 제외

각 row는 canonical path, regular-file kind, SHA-256을 가진다. 두 prompt의 요구사항 mapping에는 span도 기록하고, full-file SHA는 보조 drift evidence로 유지한다. header에는 legacy HEAD와 정확히 정렬된 git status --porcelain=v1 -z entry set의 SHA-256을 기록한다. 최종 VERIFY/SHIP가 같은 source set을 다시 발견해 additions, removals, renames, content, span, HEAD/status drift를 양방향 검사한다. 자동 rebaseline은 없다.

### Contract manifest provenance

- generator는 provenance를 명시적 input으로 받으며 ambient Git HEAD를 추론하지 않는다.
- publication 전 local/CI snapshot은 server_commit=dirty, server_tag=null을 사용한다.
- checksum은 자신의 checksum field를 제외한 deterministic artifact set을 대상으로 한다.
- drift check는 committed provenance input을 그대로 재사용한다.
- dirty workspace, clean pre-publication checkout, future publication transition fixture가 모두 byte-deterministic해야 한다.
- 미래 publication은 별도 승인 아래 두 단계다: source commit을 먼저 만들고, 다음 artifact commit/tag의 manifest가 그 source commit을 가리킨다.
- CI는 contents:read만 허용하고 PF4에서 공식 확인한 Nix installer/bootstrap action의 full commit SHA를 pin한 뒤 canonical contract-check card만 실행한다.

## 9. Nix, Rust, Just, Podman, NixOS

- 지원 system은 aarch64-darwin, aarch64-linux, x86_64-linux다.
- rust-toolchain.toml이 exact Rust release/profile/components/targets의 유일한 원본이다.
- flake.nix는 그 파일을 읽어 devShell과 package가 같은 Rust derivation을 사용하게 한다. Rust 값을 다시 적지 않는다.
- mise, rustup, .tool-versions, 두 번째 Rust version declaration은 없다.
- Justfile은 task runner일 뿐이며 tool 설치나 version pinning을 하지 않는다.
- 루트 `Justfile`이 유일한 공개 명령 목록이며 실행 전 pinned devShell이 활성화돼 있어야 한다.
- credential 생성, bounded wait, guarded deletion, service recovery처럼 안전 경계가 있는 동작만 `scripts/dev/`와 `scripts/recovery/`에 둔다.
- 현재 후속 Cargo 공유 파일 owner는 의존 순서가 보장된 task-3c(dev-only JWT), task-5(auth), task-8(S3/media)다. task-3c는 optional JWT로 dev surface를 열었고, task-5는 같은 `jsonwebtoken` verifier를 production 기본 graph로 승격하면서 PKCE Base64URL과 OAuth form/JSON feature만 추가한다. 각 owner 뒤 사용자가 lock/no-drift와 dependency/license card를 다시 실행한다.
- Task-13은 루트 `Justfile` 검증 체계 정리·통과 확인(1단계)과 실제 NixOS module(`nix/module.nix`)·package realization·runtime health smoke(2단계, `nix/smoke-test.nix`)를 모두 완료했다. homelab이 `services.jamye-server`로 이 module을 midgard에 배포했다(homelab PR #75, 2026-09-08; `homelab/services/jamye-server.nix`; 공개 URL `https://jamye-api.ridewithmin.com`, `https://jamye-media.ridewithmin.com`). 이후의 host 이전은 homelab이 독립적으로 진행하며 이 로드맵과 무관하다.
- rootless Podman compose.yaml은 local disposable PostgreSQL/Redis/MinIO 전용이다. macOS podman machine과 lifecycle/reset은 사용자가 실행한다.
- production은 native Nix package와 NixOS systemd module이다. Podman compose는 production SSOT가 아니다.
- flake는 supported system마다 api/worker package, checks, devShell, nixosModules.default를 export한다.
- api/worker package matrix는 세 system 모두를 대상으로 한다. Linux builder가 없으면 production lane blocker이며 현재-host 평가 성공을 Linux build 성공으로 기록하지 않는다.
- `just test`는 default-feature와 all-feature Rust suite를 연속 실행한다.
- `just coverage`는 library, binaries, integration targets를 포함하는 all-feature 관측 도구다. 고정 퍼센트는 구현 완료 권한이 아니며 `just check` 통과를 대신하지 않는다.
- STT worker/inference package와 관련 Nix input/config는 만들지 않는다.
- NixOS module은 package, listenAddress, environmentFile, migration policy와 선택적 `objectStorage.createLocally`를 소유한다. 로컬 개발·통합 테스트는 rootless Podman Compose를 사용하고, production에서 이 옵션을 켜면 homelab이 소비하는 module이 native `services.minio`를 함께 실행한다. D11=B에 따라 별도 bucket oneshot은 두지 않고 API `ensure_bucket`만 버킷 lifecycle을 소유한다. DB/Redis와 host/domain/volume, SOPS secret, ingress, monitoring, backup/restore는 계속 homelab 소유다.

## 10. Pending decisions

| ID | 결정 | 권고 | 현재 영향 |
|---|---|---|---|
| D1 | conversation event retention | **A no-pruning v1 (사용자 승인, locked)** | M2 schema/C0에 materialize |
| D2 | PWA Web Push coexistence | A Expo-only | 후속 RN 교체 지시로 locked; Web Push는 non-goal |
| D3 | STT/전사 범위 | C 전체 제외, voice media 보존 | 사용자 승인으로 locked non-goal |
| D4 | Apple login/Guideline 4.8 | **A: 2026-08-25에는 current server/C2 deferred였으나 2026-09-22 사용자 결정으로 구현 확정 — 상세는 D16 (locked)** | task-15가 D16을 materialize |
| D5 | account deletion sole-owner policy | **A transfer required (사용자 승인, locked)** | task-11은 이양 전 409+zero mutation을 materialize |
| D6 | rate-limit algorithm | A configurable fixed window | locked technical default |
| D7 | modular monolith | A | locked from initial prompt |
| D8 | message duplicate response shape | **A same payload 200 canonical, different payload 409 (사용자 승인, locked)** | C0에 materialize |
| D9 | notification localization representation | **A structured type+args + client localization (사용자 승인, locked)** | task-9/M8에 materialize |
| D10 | account deletion data disposition | **A tombstone/anonymize (사용자 승인, locked)** | task-11은 private state 삭제+durable object cleanup을 materialize. (2026-09-22 D15로 재개봉·확정: 계정 삭제는 30일 유예형 soft delete가 되고, tombstone/anonymize 전이는 유예 만료 후 purge 시점에 실행) |
| D11 | private bucket lifecycle owner | **B API `ensure_bucket` (사용자 승인, locked)** | task-8이 HEAD/no-op·404/create·기타 typed error를 materialize; task-13은 optional native MinIO만 제공 |
| D12 | mobile OAuth exchange flow | **A Authorization Code + PKCE S256 (사용자 승인, locked)** | M4에 materialize |
| D13 | logout/access/ticket/socket expiry | **A short token valid to exp, ticket capped by exp, socket 4401 at exp (사용자 승인, locked)** | C0에 materialize |
| D14 | soft delete 범위 | **A 전 테이블 `created_at`/`updated_at`/`deleted_at` + hard→soft 전환 + 메시지/주제 삭제 API (사용자 승인 2026-09-22, locked; 단계 분할 허용)** | task-14가 §14 단계 A-D로 materialize |
| D15 | 계정 삭제 유예·복구 | **A 30일 유예 후 기존 D10 tombstone 전이; 유예 중 삭제된 계정의 provider로 재로그인하면 계정 부활 (사용자 승인 2026-09-22, locked)** | task-14 §14 D단계가 materialize; D10의 전이 시점을 유예 만료 후로 옮김 |
| D16 | Apple login (Sign in with Apple) | **A iOS native identity token 검증, provider별 별도 계정, Android 미지원 (사용자 승인 2026-09-22, locked; D4 갱신과 동일 결정의 단일 owner)** | task-15가 Apple exchange endpoint·JWKS 검증·`auth_identities` provider 확장으로 materialize |
| D17 | Apple token revoke on account deletion (Guideline 5.1.1(v)) | 권고 A 필수 구현; task-15 착수 시 확인 | task-15가 계정 삭제 흐름과 결합해 materialize |
| D18 | 삭제 이벤트 표현 | **A 새 realtime/delta 이벤트 `message.deleted`/`topic.deleted` (사용자 승인 2026-09-22, locked)**; unknown-event 복구 규칙과 version 협상은 task-14 PLAN에서 검토 | task-14 §4 C3 계약에 materialize |
| D19 | `updated_at` 갱신 방식 | **A PostgreSQL `BEFORE UPDATE` 트리거 (사용자 승인 2026-09-22, locked)** — sqlx는 ORM이 아니라 entity lifecycle/auditing hook(JPA `@LastModifiedDate` 류)이 없으므로 한 migration의 트리거로 일괄 적용 | task-14 §14 A단계가 materialize |

현재 `pending_user` 결정은 0개다. D14-D19는 2026-09-22 사용자 승인으로 locked됐다. D5=A는 sole-owner 그룹의 소유권 이양 전 계정 삭제를 stable 409 `group_ownership_transfer_required`와 zero mutation으로 차단한다. D10=A는 공유 그룹 content/media를 author tombstone으로 익명 보존하고 credential/profile/push/notification/read/membership을 삭제하며 invite를 폐기하고 unbound upload/object를 durable cleanup으로 넘긴다. 두 결정은 2026-08-27 task-11/M10 RED 전에 사용자 승인으로 locked됐다.

D1=A, D4=A current server/C2 deferred, D8=A, D12=A, D13=A는 2026-08-25 사용자 승인으로, D11=B는 2026-08-26 사용자 승인으로, D5=A, D9=A, D10=A는 2026-08-27 사용자 승인으로 locked됐고, D2는 Expo-only, D3=C는 STT 제외로 locked다. Apple 실제 구현 또는 Guideline 4.8 예외 판정은 별도 store-release gate로 남아 있었고, 2026-09-22 사용자 결정(D4 갱신·D16)으로 task-15에서 구현하기로 확정했다.

결정 materializer는 `D1=task-3a/task-3b`, `D8/D13=task-3b`, `D12=task-5`, `D11=task-8`, `D9=task-9`, `D5/D10=task-11`로 고정한다. task-11은 locked D5=A/D10=A를 account-deletion contract/fixture/runtime evidence로 한 번 materialize하고, task-12/task-13과 VERIFY/SHIP는 그 evidence를 소비할 뿐 같은 결정을 다시 gate로 열지 않는다.

현재 push 범위는 Expo installations, notification history, canonical source-event별 durable occurrence, installation preview policy다. Web Push 관련 파일/table/Nix surface는 만들지 않는다.

`chat_unread`는 topic 대화와 group의 main chatroom 양쪽에서 만든다(2026-09-21, migration 0011). topic 메시지는 topic 단위로, main 메시지는 conversation 단위로 하나의 미읽음 notification에 coalesce하고, C3 read marker는 두 경우 모두 같은 conversation의 row를 `source_cursor` 기준으로 읽음 처리한다. `new_topic`은 topic에서만 만든다. Expo 페이로드는 항상 title/body를 담고, message preview는 installation이 허용할 때만 body를 대체한다.

D3=C에 따라 이번 작업과 C2에는 STT contract, field, job, migration, event, inference adapter/worker/provider, package, config, fixture, QA gate가 없다. 미래 STT는 새 사용자 승인과 contract/migration/worker/security/Nix/QA를 모두 소유하는 새 reviewed plan이 있어야 시작할 수 있다. 일반 voice media transport/playback은 task-8과 task-12가 보존한다.

## 11. 마일스톤과 태스크

| 순서 | 마일스톤 | Task | 산출 결과 | 시작 조건 |
|---:|---|---|---|---|
| 1 | M0 | task-1 | Rust/Nix/Just/Podman skeleton, config/logging/health | fresh PLAN PASS + 별도 M0 시작 |
| 1 | M1 | task-2 | frozen legacy capability/evidence matrix | PLAN PASS + M0 시작 이후 |
| 2 | M2 | task-3a | core schema + forward-only migration policy | task-1,2 + D1 |
| 2 | M2 | task-3b | minimal C0 contract pipeline/provenance | task-1,2 + D1,D8,D13 |
| 3 | M2b | task-3c | production-excluded dev fixture/auth harness | task-3a,3b |
| 4 | M3a | task-4a | message REST, shared TransactionHandle, atomic event/outbox, paginated delta | task-3b,3c; frozen D1 evidence 소비 |
| 4 | M4 | task-5 | production auth/profile/rate-limit + exact A3 handoff | task-3a,3b,3c + D12; frozen D13 evidence 소비 |
| 5 | M3b | task-4b | worker, Redis, authorized WS, C1 recovery | task-4a; frozen D13 evidence 소비 |
| 5 | M5 | task-6 | groups, memberships, invites | task-4a,5 |
| 6 | M5b | task-6b | chatrooms, history, read cursor | task-6 |
| 6 | M5c | task-6c | membership revoke/group delete realtime fence | task-4b,6 |
| 7 | M6 | task-7 | topics/tags/unread/announcement transaction + 0005 FK | task-6,6b |
| 8 | M7 | task-8 | private media upload/finalize/access | task-4a,6b,7 + D11 |
| 9 | M8 | task-9 | notifications, Expo push, send-authorization fence | task-6c,8 + D9 |
| 10 | M10 | task-11 | account deletion + durable object cleanup + `0008` | task-9 + locked D5=A,D10=A (충족) |
| 11 | M11a | task-12 | backend static api/worker + three UoW compositions + selected C2 | task-11; frozen decision evidence 소비 |
| 12 | M11b | task-13 | 검증 체계 정리 및 간소화, Nix flake 배포 준비 | task-12 |
| 13 | M12 | task-14 | soft delete: 전 테이블 audit 컬럼, hard→soft 전환·조회 필터, 메시지/주제 삭제 API + `*.deleted` 이벤트, 계정 삭제 유예·복구 + purge worker, C3 계약 | task-13 + D14/D15/D18/D19 |
| 14 | M13 | task-15 | Apple exchange endpoint, JWKS 검증, `auth_identities` provider 확장, 계정 삭제 시 revoke, C3 계약 | task-5, task-14(계정 삭제 결합) + D16/D17 |
| 14 | M14 | task-16 | 서버측 잔여 백로그: 메시지 편집 계약(옵션), README/roadmap drift, homelab 백업 연동, 모니터링 | task-14 |
| 15 | M15 | task-17 | 앱 M14 라운드 1 서버 지원: app-link association 공개 route, 초대 랜딩, 대화방 이미지·동영상 미디어 목록 API, 주제 미디어 제거와 `0012` migration, C2 계약 재생성 | task-13 + 2026-09-26 task-srv 승인 |
| 16 | M16 | task-18 | 앱 M14 라운드 2 서버 지원: 알림 args에 `group_name`/`topic_title`, T6 태그 교체 작성자-only, U2 `avatar_url` HTTPS 검증, Kakao `secure_resource=true`, `0013` avatar URL HTTPS migration, C2 계약 재생성 | task-17 + 2026-09-27 task-srv 승인 |

우선순위는 dependency가 없는 task는 1, 나머지는 1 + max(dependency priority)다. 같은 tier에는 dependency나 directory-prefix scope collision이 없어야 한다.

task-10은 사용자 승인 STT non-goal로 삭제했다. 기존 참조 안정성을 위해 task-11 이후 ID는 renumber하지 않아 task ID가 의도적으로 비연속이다.

서버 마일스톤 번호 M12-M16(task-14-18)는 jamye-app 로드맵의 M14-M18 번호 체계와 완전히 독립이다. task-17/18 설명의 "앱 M14 라운드"는 지원 대상 앱 라운드를 가리키며, 서버 milestone 번호를 뜻하지 않는다.

## 12. 테스트와 완료 판정

- `just check`가 format, strict Clippy, default/all-feature tests와 contract drift를 한 번씩 실행한다.
- `just check` 통과는 동결된 제품 범위의 구현 완료를 뜻한다.
- PostgreSQL/Redis stop/start는 상태를 바꾸므로 `just test-recovery`로 분리하고 release 전에 실행한다.
- coverage는 품질 관찰 지표이며 임의의 백분율이나 manifest hash가 구현 완료 권한을 갖지 않는다.
- Git history가 구현 이력을, `Cargo.lock`과 `flake.lock`이 dependency resolution을 보존한다.
- 배포 준비는 실제 NixOS module 평가, 대상 Linux package realization, service health smoke가 모두 통과해야 별도로 완료된다.

세부 명령과 부작용은 [validation](validation.md)과 [local development](development.md)에 기록한다.

## 13. 다음 단계

Task 13은 완료됐다. 검증 체계·NixOS module·homelab midgard 배포의 증거 경로와 공개 URL은 §9에 한 번만 기록한다.

2026-09-22 로드맵 등록으로 §11에 task-14-16이 추가됐다. 이 등록은 구현 승인이 아니다. R3(사용자 결정)에 따라 앱(jamye-app) M14(UI/UX 라운드)가 진행되는 동안 서버 task-14(soft delete)·task-15(Apple exchange)·task-16(잔여 백로그)을 병행 착수할 수 있다.

2026-09-26 task-17은 앱 M14 라운드 1에 필요한 서버 지원을 구현하고 같은 날 운영 배포했다. predeploy 안전 리뷰 PASS, 운영 `pg_dump`(`pre-0012-20260926T132032Z.dump`), 삭제 대상 count(주제 미디어 행·topic-scope upload·object key 모두 0)를 확인한 뒤 커밋 `9ad82e1` → PR #8 → merge `5b987a2` → homelab PR #87 배포 순서로 진행했다. migration `0012` 적용 뒤 보존 기준값이 그대로임을 확인했고, 운영 smoke는 AASA·assetlinks·`/invite/{code}` 보안 header, C5 무인증 401, MD3 404를 확인했다. 기록은 `.agents/results/deploy-20260926-181036.md`다.

2026-09-27 task-18은 앱 M14 라운드 2에 필요한 서버 지원을 구현하고 같은 날 운영 배포했다. predeploy 안전 리뷰 PASS, 운영 백업(`pre-0013-20260927T062151Z.dump`), `0013` 적용 전 avatar URL count(`http://` 1, `https://` 1)를 확인한 뒤 커밋 `d5167c3` → PR #9 → merge `a77cac5` → homelab PR #88 배포(2026-09-27 06:42:46Z 시작)로 진행했다. 적용 뒤 `http://` 0, `https://` 2, version 13 success를 확인했다. 기록은 `.agents/results/deploy-20260927-120934.md` §1-9다.

2026-09-28 앱 기기 검증 중 HTTP 메시지 전송이 첨부 2개 이상을 422 `media_not_available`로 거부하는 문제를 찾았다. 도메인 규칙·Postgres 바인딩과 계약은 이미 최대 4개를 지원했으므로 HTTP 검증(`validate_composed_http_message`)의 1개 제한만 풀어 배포했다(커밋 `9a6cba1`·`fd07187` → PR #10 → merge `c7f71a8` → homelab PR #89, 스키마·계약 변경 없음). 기록은 같은 파일 §10이다. 이때 드러난 `tests/media` fixture의 시계 오차 취약점(호스트 `now`와 DB `clock_timestamp()` 혼용으로 `media_uploads_timestamp_check`가 간헐 실패)은 후속 과제로 남긴다.

앱 M14는 2026-09-28 종료됐다. 같은 날 사용자 요청("M15 구현 시작해. 선행조건인 서버 task-14 먼저 진행하고 배포한 뒤에 시작해.")으로 task-14 착수와 배포가 승인됐다(세션 `20260928-171401`, SSOT `.agents/results/requirements-20260928-171401.md`).

2026-09-28 task-14 1차(감사 컬럼·트리거, C6/T8 삭제 API·이벤트, 계정 삭제 유예·복구, S4 push fence·realtime eviction 운영 조립, 계약 v2)는 필수 검사(base/delete/grace 각 run)와 배포 전 안전 리뷰 PASS 뒤, 운영 `pg_dump`(`pre-0014-20260928T133010Z.dump`)와 개수만 확인하는 사전 집계를 거쳐 커밋 `2618027` → PR #12 → CI `contract-drift` pass → merge `c047e5a`(13:37:15Z) → homelab PR #91 배포(14:05Z 시작) 순서로 진행했다. migration `0014`-`0016` 적용(23:10 KST, 모두 success) 뒤 행 수 불변을 확인했고, 현재 앱(v1) smoke도 정상이었다.

2026-09-28 task-14 2차(나머지 B: `push_installations`·delivery intent·`notifications`·`chatroom_reads`·`invites`·`media_uploads`·`topic_tags`의 hard→soft 전환과 live-row partial unique)는 필수 검사와 배포 전 안전 리뷰 PASS 뒤, `pg_dump`(`pre-0017-20260928T152014Z.dump`)와 사전 집계를 거쳐 커밋 `c692b62` → PR #13 → merge `b20a4d0`(15:24:54Z) → homelab PR #92 배포로 진행했다. migration `0017` 적용(2026-09-29 00:46 KST, success) 뒤 행 수 불변을 확인했고, 설치된 앱(v1) smoke도 정상이었다.

2026-09-29 task-14 3차는 기기 검증(jamye-app M15)에서 찾은 결함 — `topic.deleted`가 삭제되는 주제 대화방 피드에 기록돼 S1 403·WS 접근 필터로 아무도 받지 못하던 문제 — 를 고쳤다. `topic.deleted`를 그룹 메인 대화방 `conversation_events`에 기록하도록 옮기고(payload의 `topic_chatroom_id`는 그대로 유지), S1 v2 typed 투영 조건을 "payload `group_id`가 feed group과 같고 feed type이 `main`일 때만 허용"으로 바꿨다. 계약 discriminant·payload 필드는 바뀌지 않아 migration이 없다(pg_dump 백업도 생략, 1·2차 백업 보관 중). 필수 검사와 배포 전 안전 리뷰 PASS(LOW 1건 — 이미 삭제된 주제 대화방에 기록된 옛 `topic.deleted`는 계속 못 읽지만 회귀 아님) 뒤 커밋 `81e22d0` → PR #14 → merge `6dcb6d5`(02:04:55Z) → homelab PR #93 배포(02:21Z 시작)로 진행했다. 배포 뒤 기기 재확인: 그룹을 연 상태에서는 다른 기기의 주제 삭제가 실시간으로 반영되고, 그룹 밖에 있던 기기는 그룹을 열 때 메인 대화방 S1으로 `topic.deleted`를 받아 약 1.5초 뒤 알림 배지가 사라졌다(기존 설계상 열린 그룹만 실시간 구독).

세 배포 모두 halt 조건에 해당하는 사건이 없었고 정상 완료됐다. 전체 기록(배포 전 게이트, 운영 호스트 확인, 사전·사후 집계, 중단·복구 규칙, 운영 smoke)은 `.agents/results/deploy-20260928-171401.md` §1차/§2차/§3차다. task-15와 task-16은 이제 완료된 task-14를 선행으로 두고 각각 별도 승인으로 착수한다(§11, §14).

## 14. 후속 과제 상세

이 절은 §11 표의 task-14-18을 위한 상세 명세다. task-14-16의 상태는 `planned_unapproved`이며, 이 등록 자체는 구현·migration 적용·contract publication·homelab 배포의 승인이 아니다. task-17-18은 구현과 운영 배포를 마쳤다(§13).

### task-14 — soft delete (M12)

D14·D15·D18·D19(모두 2026-09-22 locked)를 materialize한다(앱 M15 소프트 삭제 수용과 연동). 작업 범위가 방대하므로(사용자 추가 지시: "모든 데이터에 대해 created_at, updated_at, deleted_at을 적용하고 싶은데, 작업 범위가 너무 방대하다면 쪼개도 괜찮아") 다음 네 단계로 분할할 수 있다.

- **A. audit 컬럼 migration + `updated_at` 규칙**: 대상 테이블에 `created_at`/`updated_at`/`deleted_at`을 추가하는 forward-only migration(ADR 0003, 새 번호)과 D19의 PostgreSQL `BEFORE UPDATE` 트리거를 적용한다. `refresh_sessions`, oauth attempt, `media_uploads`처럼 이미 `revoked_at`/`consumed_at`/`expires_at` 같은 의미별 컬럼을 쓰는 일회성/보안 row는 그 의미를 유지하고 `deleted_at`만 추가한다.
- **B. hard→soft 전환**: 현재 hard delete인 memberships, users, push_delivery_intents/installations, notifications·chatroom_reads·invites·refresh_sessions·auth_identities·media_uploads, topic_tags를 soft delete로 전환하고 모든 조회 경로에 `deleted_at IS NULL` 필터를 추가한다.
- **C. 삭제 API + 이벤트**: 메시지 삭제(가칭 C6)·주제 삭제(가칭 T8) REST endpoint, D18에 따른 realtime `message.deleted`/`topic.deleted` 이벤트, 삭제된 메시지/주제에 결합된 미디어 object 정리 규칙을 구현한다.
- **D. 계정 삭제 유예·복구**: D15(30일 유예, 유예 중 같은 provider 재로그인 시 부활)를 endpoint/worker로 구현하고, 유예 만료 시 기존 D10 tombstone 전이 로직을 재사용하는 purge worker를 추가한다.

구현 기록: task-14는 1·2·3차로 나눠 모두 운영 배포됐다(날짜·merge commit·migration 번호는 §0·§13). 1차(`task-srv-14a-*`)는 migration `0014`-`0016`으로 전 테이블 감사 컬럼, C6/T8 삭제 API·이벤트, 계정 삭제 유예/복구, S4 운영 조립, 계약 v2를 배포했다. 2차(`task-srv-14b`)는 남은 B 대상(`push_installations`, push delivery intents/occurrences, `notifications`, `chatroom_reads`, `invites`, `media_uploads`, `topic_tags`, 1차 밖 경로의 refresh/auth 연계)을 migration `0017`로 live-row partial unique와 `deleted_at IS NULL` 조회 필터로 전환하고, topic 공지 메시지 삭제 대상을 본문 LIKE가 아닌 `messages.announcement_for_topic_id` 구조 참조로 고정했다. 일반 사용자 동작(P4, T6/T8, C3 재생성 등)은 soft delete/live filter를 따른다. 단, 30일 유예 만료 뒤 purge는 기존 D10 의미를 유지해 unbound upload, push token/installation, refresh session, auth identity, invite, read marker, notification, target user row 같은 개인 상태를 실제 삭제한다. 3차(`task-srv-14c`)는 migration 없이, 기기 검증에서 찾은 결함(`topic.deleted`가 삭제되는 주제 대화방 피드에 기록돼 아무도 받지 못함)을 고쳐 그룹 메인 대화방 피드에 기록하도록 바꾸고 S1 v2 typed 투영 조건을 그에 맞춰 좁혔다. 1차는 새 계약을 추가했다 — C6/T8 삭제 operation, typed realtime/delta discriminant `message.deleted`/`topic.deleted`(계약 버전 v2에서만 typed, v1은 기존 `UnsupportedEventMarker` 유지), `X-Jamye-Contract-Version` `"2"`/`"1"` 협상, A2 응답 헤더 `X-Jamye-Account-Restored`(기존 REST 응답 body 모양은 설치된 v1 앱을 위해 닫힌 채로 유지). 2차는 새 operation·필드·discriminant를 추가하지 않고, `openapi.info.version`이 `CURRENT_CONTRACT_VERSION`을 실제로 반영하도록 표기 버그만 고쳤다. 3차도 새 operation·필드·discriminant 없이 `topic.deleted` 기록 위치와 S1 v2 투영 조건만 바꿨다(계약 산출물은 바이트 단위로 불변, `contract-check` 통과). 3차 배포 전 주제 대화방 피드에 기록된 `topic.deleted` 8건은 계속 읽을 수 없다(배포 전 리뷰에서 수용, backfill 없음).

후속 과제(사용자 결정 2026-09-29): 계정이 삭제된 사람이 보낸 삭제 표시 메시지 행은 그 사람의 살아 있는 메시지(C2)나 작성한 주제(T3/T4)를 다시 받아야 이름·사진이 갱신된다(앱 jamye-app 로드맵 M15 "후속 후보"와 jamye-app 저장소의 M15 증거 문서 §7 참고). 완전한 해결은 서버 계약 확장이 필요하다 — 예: 삭제된 메시지의 보낸 사람 identity를 위한 별도 source(버전 있는 C2의 sender stub 등)나 identity-change 신호. M15 범위가 아니라 후속 과제로 미룬다. 30일 뒤 purge의 과거 payload scrub(`src/adapters/postgres/account_deletion/payload_scrub.rs`)이 색인 없는 `payload::text LIKE`를 쓴다 — 테이블이 커지기 전 색인 추가 또는 대상 이벤트 id 추적으로 바꿔야 한다.

시작 조건: task-13 완료(충족) + D14/D15/D18/D19 확정(충족). 별도 승인(모두 충족·완료): 사용자가 2026-09-28 착수·배포를 승인했고, 각 forward-only migration 적용·C3 계약 publication·homelab 배포를 1·2·3차 모두 마쳤다(§0, §13).

### task-15 — Sign in with Apple (M13)

D16(locked)·D17(권고 필수)을 materialize한다(앱 M16 Sign in with Apple과 연동). Apple exchange endpoint(가칭 A6), Apple identity token 서명·`iss`·`aud`(bundle id allowlist)·`exp`·nonce 검증, `auth_identities` provider 확장(`apple` 추가), config `JAMYE_APPLE_*`(Team ID, Key ID, `.p8` 경로, bundle id allowlist), 계정 삭제 시 Apple token revoke 호출(D17, Guideline 5.1.1(v))을 구현하고 C3 계약에 반영한다.

시작 조건: task-5(OAuth 기반, 충족) + task-14(계정 삭제 흐름과 결합, D17) + D16 확정(충족)·D17은 task-15 착수 시 확인. 별도 승인: Apple Developer 설정(App ID Sign in with Apple capability, key(.p8)/Team ID/Key ID)은 사용자가 직접 수행, migration 적용, C3 계약 publication, homelab 배포.

### task-16 — 서버측 잔여 백로그 (M14)

요구사항 R8(서버·운영 묶음)에 대응한다. 항목: 메시지 편집 계약(옵션, 앱 M17(C)와 연동), README/roadmap drift 정리, homelab 자동 백업 연동, 모니터링·알림 점검(앱 M17(D) 서버·운영 묶음과 연동).

시작 조건: task-14 완료. 별도 승인: 각 항목은 개별 승인으로 착수하며, homelab 변경이 필요한 항목(백업 연동)은 homelab 로드맵과 별도로 조율한다.

### task-17 — 앱 M14 라운드 1 서버 지원 (M15)

상태: `deployed` (2026-09-26 구현·운영 배포, merge `5b987a2`, migration `0012`). 이 task는 앱 M14 라운드 1 요구사항 중 서버가 선행해야 하는 S1-S3를 구현한다. 공개 route는 `GET /.well-known/apple-app-site-association`, `GET /.well-known/assetlinks.json`, `GET /invite/{code}`이고, 기본 association 값은 개발 bundle/package와 Android debug SHA-256(E3)을 사용한다. 잘못된 초대 코드는 404이며, 형식이 맞는 코드도 DB 조회 없이 정적 HTML만 반환해 그룹 이름·유효성 oracle을 노출하지 않는다.

대화방 갤러리는 `GET /api/v1/chatrooms/{chatroom_id}/media`(`C5`)로 제공한다. membership은 기존 chatroom 권한 경로를 재사용하고, item cursor는 `message_media.id`이며 정렬은 메시지 작성 시각 최신순, 메시지 id 역순, 첨부 position 오름차순이다. 응답은 기존 `MessageAttachment` 필드와 `message_id`, `message_created_at`을 포함하고 이미지·동영상만 반환한다.

주제 미디어는 서버까지 제거했다. `GET /api/v1/topics/{topic_id}/media`, topic-scope upload/finalize, `CanonicalTopic.media`, `UploadFinalizeResult` topic 분기를 계약과 코드에서 제거하고, `migrations/0012_remove_topic_media.sql`가 기존 topic media row와 topic-scope upload를 제거한다. `account_object_deletion_intents`가 임의 object key를 받을 수 있으므로 topic object는 기존 object cleanup worker 경로로 삭제 예약한다. 배포 전 운영 읽기 전용 count SQL로 삭제 대상 행·object 수가 모두 0임을 기록했다(`.agents/results/deploy-20260926-181036.md` §3).

새 env 기본값은 `JAMYE_APP_LINKS_AASA_APP_IDS=6ZH8V43A7D.dev.local.jamyeapp`, `JAMYE_APP_LINKS_ANDROID_PACKAGE=dev.local.jamyeapp`, `JAMYE_APP_LINKS_ANDROID_SHA256_CERT_FINGERPRINTS=FA:C6:17:45:DC:09:03:78:6F:B9:ED:E6:2A:96:2B:39:9F:73:48:F0:BB:6F:89:9B:83:32:66:75:91:03:3B:9C`, `JAMYE_APP_STORE_URL=`(빈 값), `JAMYE_PLAY_STORE_URL=`(빈 값)이다. 스토어 URL은 설정 시 `https`만 허용한다. 설계 결정은 [ADR 0010](adr/0010-app-links-chatroom-media-topic-media-removal.md)에 기록한다.

### task-18 — 앱 M14 라운드 2 서버 지원 (M16)

상태: `deployed` (2026-09-27 구현·운영 배포, merge `a77cac5`, migration `0013`). 이 task는 앱 M14 라운드 2 요구사항 중 서버 S1-S4를 구현한다. 알림 fan-out은 기존 `NotificationArgs` map 안에 `group_name`과 주제 관련 알림의 `topic_title`을 추가한다. `new_topic`은 `author_display_name`, `group_name`, `topic_title`을, 주제 대화방 `chat_unread`는 `sender_display_name`, `group_name`, `topic_title`을, 그룹 기본 대화방 `chat_unread`는 `sender_display_name`, `group_name`을 가진다. Expo push delivery payload는 기존 `{type, notification_id, conversation_id, message_id}` 모양을 유지한다.

T6 태그 교체 권한은 작성자-only로 맞췄다. 그룹 소유자라도 주제 작성자가 아니면 기존 `403 topic_author_required` 오류를 받으며, 계약 contribution의 T6 auth도 `author`로 갱신한다.

U2 `PATCH /api/v1/me`의 `avatar_url`은 null/omitted를 변경 없음, 빈 문자열을 삭제, 그 외 문자열을 길이 512 이하의 절대 `https` URL(host 필수)로 검증한다. Kakao identity 요청은 `secure_resource=true`를 보낸다. Provider가 준 avatar URL은 `http://`이면 `https://`로 변환한 뒤 같은 규칙으로 검증하고, 통과하지 못하면 저장하지 않는다.

`migrations/0013_https_avatar_urls.sql`은 저장된 `http://` avatar URL만 `https://`로 변환하는 forward-only migration이다. 운영 배포 전후 읽기 전용 확인 SQL은 다음 세 개다.

```sql
SELECT count(*) FROM users WHERE avatar_url LIKE 'http://%';
SELECT count(*) FROM users WHERE avatar_url LIKE 'https://%';
SELECT version, success FROM _sqlx_migrations WHERE version = 13;
```

배포 결과: 적용 전 `http://` 1·`https://` 1, 적용 뒤 `http://` 0·`https://` 2, version 13 success(`.agents/results/deploy-20260927-120934.md` §3·§7).
