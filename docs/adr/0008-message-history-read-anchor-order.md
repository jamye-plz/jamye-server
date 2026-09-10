# ADR 0008: C2 history order and C3 message anchors

- 상태: Accepted
- 날짜: 2026-09-10
- 배경: [PR #2 순서 지적](https://github.com/jamye-plz/jamye-server/pull/2#discussion_r3975462936), [조회 인덱스 지적](https://github.com/jamye-plz/jamye-server/pull/2#discussion_r3975462942)

## 문제

메시지의 `created_at`과 이벤트의 `cursor`는 서로 다른 INSERT에서 할당된다.
기존 shared authorization lock은 두 전송을 동시에 허용하므로 A의 메시지가
먼저 저장되고 B의 이벤트가 먼저 저장될 수 있었다. C2의 마지막 메시지를
C3에 넘겨도 그 앞의 메시지가 unread로 남을 수 있는 조건이다.

또한 C3의 `payload ->> 'id'` 조건은 기존 conversation/cursor 인덱스만으로
메시지 하나를 직접 찾을 수 없었다.

## 결정

- API 계약과 C2 `(created_at, id)` 정렬·페이지 규칙은 변경하지 않는다.
- 일반 메시지 전송과 토픽 생성의 main-chat 안내 메시지는 공통 PostgreSQL
  `message_order::next_timestamp`를 사용한다. production INSERT 경로는 이 둘이다.
- 기존 group/membership authorization lock 다음에 채팅방별 transaction advisory
  lock을 획득한다. 메시지·이벤트·outbox·관련 변경이 같은 caller-owned transaction에서
  commit/rollback될 때까지 유지한다. helper가 transaction을 새로 열지 않는다.
- lock 획득 후 **별도 READ COMMITTED 쿼리**로 직전 메시지를 조회하고,
  `max(clock_timestamp(), 직전 created_at + 1 microsecond)`를 저장한다.
  따라서 동시 전송뿐 아니라 clock rollback과 timestamp tie에도 두 순서가 일치한다.
- 토픽 생성의 기존 group exclusive lock을 먼저 유지한다. room lock부터 얻고
  group lock으로 승격하는 경로를 만들지 않는다. 다른 채팅방은 독립적으로 진행한다.
- 논리적 메시지→canonical event 관계, 정확히 하나의 이벤트를 요구하는 C3 검증,
  기존 읽음 cursor의 단조 증가 규칙은 유지한다.
- 물리적 최적화로 migration `0009`가 non-unique partial expression index
  `(conversation_id, payload ->> 'id') INCLUDE (cursor)`를 추가한다.
  대상은 `message.created`, version 1뿐이다. 기존 payload나 timestamp는 수정하지 않는다.

## 배포와 복구

기존 [transactional forward-only migration 정책](0003-forward-only-sqlx-migrations.md)을
유지한다. `0009`는 lock 대기를 5초, 각 statement 실행을 60초로 제한한다.
인덱스 생성 중 해당 테이블의 write는 대기할 수 있다. 실패하면 인덱스와 SQLx
기록이 함께 rollback되므로 원인을 해소한 뒤 같은 migration을 재시도할 수 있다.
큰 기존 DB에서는 배포 전 event 수·테이블 크기·긴 transaction을 확인하고 유지보수
시간을 잡아야 한다. timeout을 무조건 늘리거나 기록을 수동 조작하지 않는다.

현재 homelab은 수정 전 read-only 사전 확인에서 메시지가 0건이었다.
기존 메시지가 있는 다른 환경은 과거 history/event 순서 불일치를 별도 조사해야 한다.
이번 수정은 앞으로의 협력하는 writer 순서를 보장하며 과거 데이터를 재정렬하지 않는다.
직접 SQL 또는 다른 writer를 추가할 때도 같은 lock 순서를 따라야 한다.

애플리케이션 rollback 시 인덱스는 남겨도 이전 코드와 호환된다. 단 이전 binary를
다시 사용하면 전송 순서 버그도 돌아오므로 forward-fix를 우선한다. 데이터 삭제나
down migration은 이 변경의 복구 절차에 포함하지 않는다.

## 검증과 비용

- 첫 메시지 INSERT 직후 test-only trigger를 멈추는 결정적 interleaving 테스트:
  기존 코드에서 순서 불일치 재현, 수정 후 C2 history와 event cursor 및 C3 일치.
- 다른 방의 진행, 미래 timestamp 이후 일반 전송·토픽 안내, 기존 중복 전송·rollback 검증.
- exact v8 upgrade, migration 재실행, 강제 실패 후 rollback 및 재시도 검증.
- 실제 C3 JOIN/ORDER BY/FOR SHARE 쿼리의 기본 planner로 비교한다. 6,001개 event의
  로컬 샘플에서 event scan 6,001행 → index lookup 1행, shared buffers 230 → 7.
  인덱스 크기는 532,480 bytes였다. 이 수치는 로컬 샘플이며 운영 latency/SLA가 아니다.
- 각 canonical event write에 index 유지 비용이 추가되고, 같은 방의 write 처리량은
  transaction 완료 속도에 제한된다. 실측 전 특정 동시 사용자 수를 보장하지 않는다.
