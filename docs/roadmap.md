# jamye-server 로드맵 — FastAPI 전체 이관, 신뢰성 고도화, 모바일 계약

> 세션: ultrawork/20260822-200110
> 현재 단계: PLAN_GATE passed; M0·M1·M2·M3a·M3b·M4·M5(task-6) 완료
> 상태: task-6 GREEN·guarded Redis stop/restart recovery 통과; 다음 task 준비 대기
> 기계 SSOT: .agents/results/plan-20260822-200110.json

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
- 구현 중 consequential local/dev 명령도 사용자가 직접 실행한다. 루트 Justfile은 task module만 등록하고, 각 feature owner는 자기 module과 `docs/commands/<task-id>/`에 명령, 목적, 부작용, 예상 결과, 복구 방법을 기록한다. 별도 Bash는 credential/trap/wait/guarded deletion처럼 실제 안전 경계를 제공할 때만 둔다.
- 모든 command card는 nix develop path:. 안에서 실행한다. 계획 문서에 shell body를 중복하지 않는다.
- pending 제품 결정은 가장 이른 materializer가 한 번만 사용자 선택을 받아 evidence를 고정한다. 후속 task와 VERIFY/SHIP는 dependency를 통해 그 evidence를 소비하며 같은 결정을 다시 승인받지 않는다.
- production/release/SCM 변경은 별도 승인이 있어야 한다.
- legacy jamye-plz, homelab, 운영 PostgreSQL/Redis/MinIO는 읽기 전용 또는 범위 밖이다.

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

C0는 정확히 5개 REST operation(H1, H2, C4, S1, R1)과 message.created 하나만 생성한다. D1, D8, D13만 C0를 막을 수 있다. 인증, 그룹, 주제, 미디어, 알림, 푸시, 계정 삭제 계약은 각 runtime feature owner가 나중에 추가한다.

후속 PWA→React Native 교체 지시로 D2는 Expo-only로 고정되었다. 선택된 최종 REST index는 정확히 42행이며, Web Push/VAPID 관련 runtime/schema/table/config/Nix surface는 만들지 않는다.

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
- 각 worker owner가 timeout/lease 값을 feature-local non-secret config로 정의·검증하고, task-13은 최종 `.env.example`과 NixOS module에 그 값을 노출한다.

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
- disposable local reset은 guarded command card로 문서화한다.
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

- 지원 system은 aarch64-darwin development와 x86_64-linux production이다.
- rust-toolchain.toml이 exact Rust release/profile/components/targets의 유일한 원본이다.
- flake.nix는 그 파일을 읽어 devShell과 package가 같은 Rust derivation을 사용하게 한다. Rust 값을 다시 적지 않는다.
- mise, rustup, .tool-versions, 두 번째 Rust version declaration은 없다.
- Justfile은 task runner일 뿐이며 tool 설치나 version pinning을 하지 않는다.
- 모든 command card는 nix develop path:.에서 실행한다.
- task-1은 M0/C0에서 실제 쓰는 dependency, task-1 Just module, base `.env.example`만 고정한다. 미래 feature graph나 behavior recipe를 미리 만들지 않는다.
- 현재 후속 Cargo 공유 파일 owner는 의존 순서가 보장된 task-3c(dev-only JWT), task-5(auth), task-8(S3/media)다. task-3c는 optional JWT로 dev surface를 열었고, task-5는 같은 `jsonwebtoken` verifier를 production 기본 graph로 승격하면서 PKCE Base64URL과 OAuth form/JSON feature만 추가한다. 각 owner 뒤 사용자가 lock/no-drift와 dependency/license card를 다시 실행한다.
- task-13은 `flake.nix`, `nix/`, module evaluation, `.env.example`, `docs/deployment-nix.md`, task-owned command docs/module/scripts만 소유한다. 루트 README/Justfile에 개별 feature recipe를 추가하지 않는다.
- rootless Podman compose.yaml은 local disposable PostgreSQL/Redis/MinIO 전용이다. macOS podman machine과 lifecycle/reset은 사용자가 실행한다.
- production은 native Nix package와 NixOS systemd module이다. Podman compose는 production SSOT가 아니다.
- flake는 supported system마다 api/worker package, checks, devShell, nixosModules.default를 export한다.
- api/worker package matrix는 두 system 모두를 대상으로 한다. x86_64-linux builder가 없으면 production lane blocker이며 skip 성공으로 기록하지 않는다.
- default-feature Rust tests와 all-feature Rust tests는 별도 card다.
- coverage card는 구현 시 공식 확인한 all-target command로 library, binaries, integration targets를 모두 포함해 80% 이상을 요구한다.
- STT worker/inference package와 관련 Nix input/config는 만들지 않는다.
- NixOS module은 package, listenAddress, environmentFile, migration policy만 소유한다. DB/Redis/MinIO 서비스, host/domain/volume, SOPS secret, ingress, monitoring, backup/restore는 homelab 소유다.

## 10. Pending decisions

| ID | 결정 | 권고 | 현재 영향 |
|---|---|---|---|
| D1 | conversation event retention | **A no-pruning v1 (사용자 승인, locked)** | M2 schema/C0에 materialize |
| D2 | PWA Web Push coexistence | A Expo-only | 후속 RN 교체 지시로 locked; Web Push는 non-goal |
| D3 | STT/전사 범위 | C 전체 제외, voice media 보존 | 사용자 승인으로 locked non-goal |
| D4 | Apple login/Guideline 4.8 | **A current server/C2에서는 deferred (사용자 재확인, locked)** | task-5는 Kakao/Google만 구현; Apple은 store-release gate |
| D5 | account deletion sole-owner policy | A transfer required | M10 전 필요 |
| D6 | rate-limit algorithm | A configurable fixed window | locked technical default |
| D7 | modular monolith | A | locked from initial prompt |
| D8 | message duplicate response shape | **A same payload 200 canonical, different payload 409 (사용자 승인, locked)** | C0에 materialize |
| D9 | notification localization representation | A structured type+args | M8 전 필요 |
| D10 | account deletion data disposition | A tombstone/anonymize | M10 전 필요 |
| D11 | private bucket lifecycle owner | A homelab admin provision | M7 전 선택; M11b는 frozen evidence 소비 |
| D12 | mobile OAuth exchange flow | **A Authorization Code + PKCE S256 (사용자 승인, locked)** | M4에 materialize |
| D13 | logout/access/ticket/socket expiry | **A short token valid to exp, ticket capped by exp, socket 4401 at exp (사용자 승인, locked)** | C0에 materialize |

현재 pending_user는 D5, D9, D10, D11의 4개다. 에이전트가 임의 선택하지 않는다. D1=A, D4=A current server/C2 deferred, D8=A, D12=A, D13=A는 2026-08-25 사용자 승인으로 locked됐고, D2는 Expo-only, D3=C는 STT 제외로 locked다. Apple 실제 구현 또는 Guideline 4.8 예외 판정은 별도 store-release gate로 남는다.

결정 materializer는 `D1=task-3a/task-3b`, `D8/D13=task-3b`, `D12=task-5`, `D11=task-8`, `D9=task-9`, `D5/D10=task-11`로 고정한다. task-4a/task-4b/task-12/task-13과 VERIFY/SHIP는 이미 고정된 evidence를 소비할 뿐 같은 결정을 다시 gate로 열지 않는다.

현재 push 범위는 Expo installations, notification history, canonical source-event별 durable occurrence, installation preview policy다. Web Push 관련 파일/table/Nix surface는 만들지 않는다.

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
| 10 | M10 | task-11 | account deletion + durable object cleanup + `0008` | task-9 + D5,D10 |
| 11 | M11a | task-12 | backend static api/worker + three UoW compositions + selected C2 | task-11; frozen decision evidence 소비 |
| 12 | M11b | task-13 | dual-system packages/checks/NixOS module/docs | task-12 |

우선순위는 dependency가 없는 task는 1, 나머지는 1 + max(dependency priority)다. 같은 tier에는 dependency나 directory-prefix scope collision이 없어야 한다.

task-10은 사용자 승인 STT non-goal로 삭제했다. 기존 참조 안정성을 위해 task-11 이후 ID는 renumber하지 않아 task ID가 의도적으로 비연속이다.

## 12. 테스트와 증거의 소유권

- 각 feature task가 자신의 RED/GREEN behavior를 소유한다.
- migration owner가 자신의 transactional up/upgrade/rollback evidence를 소유한다.
- task-12/backend는 task-4a의 shared handle을 소비해 final static api/worker composition, 세 개 UoW의 직접 조합, selected operation → handler/test/fixture reachability, deterministic C2 generation/provenance와 한 개의 실제 cumulative rollback integration target을 구현한다. transaction port/SQLx adapter는 소유하지 않는다.
- task-12는 feature behavior를 다시 구현하거나 final QA authority가 되지 않는다.
- VERIFY/SHIP만 PF1 재발견, task-2 최종 inventory/matrix, migration metadata/full chain, log/secret/license/package/security와 whole-tree regression을 감사한다.
- secret scanner sentinel self-test는 M0에서 한 번 실행한다. SHIP에서는 final clean working-tree scan만 실행한다.
- exact command는 task-owned Just module에, 목적·부작용·복구는 `docs/commands/`에 둔다. 별도 `scripts/tasks/`는 안전상 독립 script가 필요한 동작만 소유하고 루트 README/Justfile에는 module catalog만 존재한다.

최종 VERIFY/SHIP 범주는 다음과 같다.

- one final-verify dispatcher path: format/lint/default+all-feature/all-target tests/architecture/migration/contract/coverage
- PF1 frozen-source equality와 정확히 42 REST/2 realtime inventory audit
- PostgreSQL/Redis/MinIO outage + paginated delta recovery
- push privacy/send-authorization interleaving
- worker short-I/O timeout/lease/reclaim/stale-discard
- runtime secret log + dependency/license/advisory + final clean secret scan
- aarch64-darwin/x86_64-linux api/worker package + NixOS module evaluation
- bodyless exactly-one-audio voice의 atomic send, history/delta/message.created, authorized presigned-GET 재발급 evidence

외부 서비스나 Linux builder가 없으면 조용히 skip하지 않는다. 필요한 조건과 blocker를 기록한다.

## 13. 다음 단계

구조 계획은 SHA-256 `3961a5108d4fb384d7e92e7b9fdaeca4c96e8e303acea8fa330b0f393679c973`에서 fresh r16 completeness, meta, simplicity review를 material finding 0으로 통과했다. 이후 사용자 선택 D1=A, D4=A current server/C2 deferred, D8=A, D12=A, D13=A의 evidence를 추가한 현재 실행 snapshot은 SHA-256 `ff90657984a0eb601fa9890ba96dc041ba4a9ac4320022eb707a8db3f9750d87`이다. Task, dependency, contract count는 바뀌지 않았고 이미 점유된 ADR `0003`/`0004`를 보존하기 위해 task-5 ADR artifact path만 `0005`/`0006`으로 교정했다.

다음 순서는 고정한다.

1. task-1 M0 evidence는 commit `d285e75`까지 기준선으로 기록됐다.
2. task-2는 PF1 89-file manifest와 40 operation·13 table·189 test·6 migration 양방향 matrix를 동결했다.
3. 사용자가 L08=A를 선택해 P4를 installation-specific delete로 잠갔고 M1_SCOPE_LOCK을 통과했다.
4. 사용자가 D1=A, D8=A, D13=A를 승인했다. task-3a는 no-pruning event schema와 lease-generation outbox foundation을 materialize했고, task-3b는 동일 payload 200/different payload 409 및 token-exp/ticket/socket lifetime을 C0에 materialize했다.
5. 사용자가 D4=A와 D12=A를 승인해 task-5의 Kakao/Google system-browser Authorization Code + PKCE S256, Redis digest-only 10분 one-time attempt 경계를 잠갔다. Apple 실제 구현은 별도 store-release gate다.
6. 이후에는 dependency, earliest materializer가 고정한 decision evidence, user-run evidence가 충족되면 별도 milestone-start 승인 없이 진행한다. production/release/추가 SCM 작업은 계속 별도 승인을 요구한다.

아래 장문 단락은 task-5 RED 준비 시점까지의 누적 실행 이력을 보존한 historical
snapshot이다. 현재 실행 상태는 문서 머리말과 task-owned command evidence가 우선하며,
task-5는 2026-08-26 GREEN과 Redis recovery를 마치고 task-6 RED 준비로 전환됐다.

task-1, task-2, task-3a, task-3b, task-3c, task-4a, task-4b는 completed이고 task-5는 D12=A 결정 materialization과 RED gate 준비 상태이며 후속 9개 task는 pending이다. 계획 문서와 `.serena/project.yml`은 commit `a86d51c`로 기록돼 있으며, M0 변경은 사용자 승인에 따라 기준선 commit `0b1b04b`와 후속 보정 commit들로 기록했다. 사용자 실행으로 provider/toolchain, Cargo/Nix lock no-drift, dependency advisory/ban/license/source, working-directory secret-scan clean/detect/clean, Cargo format/Clippy/default+all-feature+architecture test gate까지 통과했다. 사용자 결정에 따라 generic script dispatcher를 task-1 Just module과 safety-only Bash 경계로 단순화했고, Just 1.58.0 parser/format 및 Bash syntax 정적 검증을 통과했다. rootless Podman에서 PostgreSQL 17.11, Redis 8.10.1, MinIO RELEASE.2025-09-07T16-13-09Z가 loopback binding으로 모두 healthy임을 확인했다. 실행 중인 API의 `/health/live`는 `live`, `/health/ready`는 PostgreSQL 필수 및 Redis/MinIO 선택 의존성이 모두 `ready`임을 반환했다. worker와 API가 Test 환경에서 정상 시작했으며 `Ctrl-C` 뒤 각각 종료 로그를 남겼다. local flake show에서 aarch64-darwin과 x86_64-linux의 package/devShell/check shape를 확인했고 aarch64-darwin flake check가 통과했다. 사용자가 구성한 `linux-builder-vz`를 통해 x86_64-linux API와 worker를 실제 빌드해 `/nix/store/sqhc2lpyly8p9w7mdwyib21df3rsxaq4-jamye-server-0.1.0`, `/nix/store/fifjz6gkpm32g7nvc4ki1h7iljg5yr02-jamye-server-0.1.0` output을 생성했으며 `flake_linux_exit=0`을 확인했다. task-2는 legacy 저장소를 수정하지 않고 PF1 inventory, behavior/discrepancy matrix, target reverse coverage를 동결했으며 사용자의 L08=A 선택을 `approved_by_user_2026-08-25_option_A`로 기록했다. task-3a는 빈 disposable PostgreSQL에서 `0001` 적용, 7개 core table/constraint/index, server-generated cursor, typed outbox defaults, forced-failure 전체 rollback을 사용자 실행 GREEN 6/6과 exit `0`으로 검증했다. task-3b는 C0 5개 REST operation과 `message.created` realtime contract를 16개 deterministic artifact로 생성했으며, 사용자 실행 테스트 5/5, committed/temp tree 검증, provenance/checksum 및 byte-for-byte drift 검사 exit `0`을 확인했다. task-3c는 예상한 구현 부재 RED exit `101` 뒤 사용자 승인 A로 optional `jsonwebtoken 11.0.0` 경계를 선택했다. 첫 lock/no-drift exit `0`은 `rust_crypto` graph를 고정했지만 이어진 dependency card가 사용하지 않는 `rsa 0.9.10`의 `RUSTSEC-2023-0071`로 exit `1`을 반환했다. 예외 등록 대신 공식 `aws_lc_rs` backend로 교정했고, 사용자 재실행 lock/no-drift exit `0`으로 `Cargo.lock` SHA-256 `1dce2310998050f3f00e8dd418f169d36ddbc4e91538e2815beedaff4386d87e` 및 기존 `flake.lock` SHA-256 `31403f6a698d7386579ca297f53952fd8cb47616affa8ff49c9fc71517f05bd9`를 고정했다. 새 lock에서 미사용 RSA/ECDSA/EdDSA package는 제거됐고, 재실행 dependency card는 중복 경고만 남긴 채 advisories/bans/licenses/sources 모두 ok와 exit `0`을 반환했다. 첫 GREEN은 SQLx macro feature mismatch로 exit `101`이었으나 기존 runtime `Migrator` 경계로 교정한 뒤 기본 graph 1/1, guarded unit 1/1, PostgreSQL 통합 5/5, architecture 4/4와 최종 exit `0`을 확인했다. task-3c 구현은 `8d2e0ad`, 최종 crypto backend 문서 보정은 `8c97924`로 기록했다. task-4a는 user-run RED exit `101` 뒤 메시지 REST, 단일 shared TransactionHandle, atomic message/event/outbox, D8 idempotency와 paginated delta를 구현했다. PostgreSQL 예약 키워드 CTE 결함을 focused diagnostic으로 찾아 수정한 뒤 GREEN messaging 10/10, architecture 4/4, 주입형 PostgreSQL outage/recovery와 structured-log redaction 2/2가 exit `0`으로 통과했다. 이어 실제 guarded Compose PostgreSQL을 중지·재시작하면서 같은 in-process Router가 liveness 200, readiness와 C4/S1 safe 503, 재연결 후 partial row 0개, 같은 `client_msg_id`의 message/event/outbox 각 1개 commit을 증명했고 actual lifecycle 1/1 및 최종 exit `0`을 확인했다. task-4b RED gate는 production surface와 Cargo/lockfile을 건드리지 않은 채 사용자 실행에서 예상한 surface 부재와 exit `101`을 확인했다. 그 뒤 bounded Axum WebSocket/CSPRNG/SHA-256/test-client dependency만 추가했고, 사용자 실행 lock/no-drift가 `Cargo.lock` SHA-256 `de00bfd644191e367eaa4979940c19655d87464d23551776e8f08ac336fa4a5e`, 기존 `flake.lock` SHA-256 `31403f6a698d7386579ca297f53952fd8cb47616affa8ff49c9fc71517f05bd9`, exit `0`을 확인했다. dependency card도 중복 crate 경고만 남긴 채 advisories/bans/licenses/sources 모두 ok와 exit `0`을 반환했다. 첫 GREEN compile은 `tungstenite 0.29` `Utf8Bytes`의 다중 `AsRef` 구현 때문에 close reason assertion 5곳에서 E0283으로 실패했으나 비교를 `as_str()`로 명시한 뒤 realtime 21/21과 architecture 4/4가 통과해 `task_4b_green_exit=0`을 확인했다. 이어 guarded Redis container만 실제로 중지·재시작한 recovery target 1/1이 같은 Router/worker의 복구와 PostgreSQL outbox byte 보존을 증명했고 `task_4b_redis_recovery_exit=0`으로 완료됐다. 사용자가 D12=A를 선택해 task-5의 Authorization Code + PKCE S256, digest-only 10분 OAuth attempt 경계를 잠갔으며 production source나 dependency를 추가하기 전 compile-only RED card를 준비했다. production deployment/migration은 계속 별도 승인 범위다.
