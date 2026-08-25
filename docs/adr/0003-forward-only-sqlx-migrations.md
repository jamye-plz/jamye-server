# ADR 0003: Forward-only SQLx migrations

- 상태: Accepted
- 날짜: 2026-08-25
- 범위: `jamye-server`의 PostgreSQL schema migration

## Context

PostgreSQL은 사용자·그룹·메시지·동기화 이벤트·outbox의 authoritative state다. 따라서 migration 실패나 순서 오류가 persistent database에 partial schema를 남기거나, 운영 데이터에 추측성 down migration을 적용하게 해서는 안 된다.

첫 migration은 빈 database에서 시작하지만 이후 migration은 바로 앞 번호의 실제 schema를 전제로 한다. 특히 `0001`은 아직 존재하지 않는 `topics`를 참조할 수 없으므로 `chatrooms.topic_id`의 UUID column, type CHECK, partial uniqueness만 만든다. `0005`가 `topics`를 만든 뒤 foreign key를 추가한다.

## Decision

### 번호와 source control

- production migration은 `migrations/NNNN_<name>.sql`의 단조 증가하는 단일 sequence로 관리한다.
- 이미 공유되거나 적용된 migration 파일은 수정·재정렬·삭제하지 않는다. 변경은 새 번호의 forward-fix migration으로만 표현한다.
- speculative down migration 파일은 만들지 않는다.
- 한 task가 둘 이상의 경쟁 migration branch를 만들지 않는다. 번호의 immediate predecessor가 준비되지 않았으면 후속 task는 시작하지 않는다.

### migration metadata

모든 migration 첫머리에는 다음 metadata를 둔다.

```sql
-- migration: NNNN_name
-- prerequisite: exact prior numbered schema or empty database
-- reversibility: forward-only; repair or evolve with a new numbered migration
-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md
-- rationale: migration-local reason for this schema change
```

`reversibility`는 단순 표식이 아니라 destructive change 여부와 안전한 forward-fix 방향을 reviewer가 판단할 수 있게 migration-local rationale과 함께 유지한다. production restore/import/cutover 절차를 이 metadata에 즉석으로 넣지 않는다.

### transaction과 검증

- SQLx의 기본 transactional migration을 사용한다. `-- no-transaction`은 금지한다.
- 각 migration owner는 disposable PostgreSQL에서 다음을 증명한다.
  1. exact immediate-prior schema에서 해당 번호까지 upgrade된다.
  2. 기대 table, column, constraint, index가 존재하고 잘못된 write를 거부한다.
  3. migration 끝에 강제 오류를 붙이면 migration 전체가 rollback되어 partial schema가 남지 않는다.
- `0001`의 immediate-prior state는 application table이 하나도 없는 빈 database다.
- task-12와 최종 VERIFY는 별도로 canonical `0001`부터 `0008`까지의 fresh chain과 누적 upgrade path를 검증한다. 일부 번호만 persistent 또는 production database에 적용하지 않는다.

PostgreSQL identity column의 `GENERATED ALWAYS`는 client가 conversation cursor를 일반 INSERT로 덮어쓰지 못하게 하며, cursor에는 별도 UNIQUE constraint를 둔다. Partial UNIQUE index는 조건을 만족하는 row에만 uniqueness를 적용하므로 main chatroom, topic chatroom, user message idempotency key의 제한된 유일성에 사용한다.

### local disposable recovery

일반 GREEN test는 `JAMYE_ENVIRONMENT=test`, loopback PostgreSQL, base database `/jamye_test`를 모두 확인한 뒤 `jamye_task_<task>_<uuid>` database만 생성하고 exact name만 제거한다. 임의 host, persistent database, production database에서는 실행을 거부한다.

test process가 중단되어 disposable database가 남았다고 의심될 때는 먼저 exact database name과 prefix를 확인한다. 확인 없이 database나 Podman volume을 삭제하지 않는다. 전체 local volume reset이 정말 필요하면 [task-1 local infrastructure card](../commands/task-1/local-infrastructure.md)의 guarded `infra-reset`을 사용자가 명시적으로 선택한다. 이 명령은 migration test의 기본 복구 수단이 아니다.

## Data ownership and operational boundary

- PostgreSQL schema와 numbered migration은 이 repository가 소유한다.
- Redis와 MinIO는 migration 대상이 아니다. Redis 유실은 committed message/event/outbox row의 손실을 뜻하지 않는다.
- backup schedule, restore drill, production PostgreSQL service와 volume은 `homelab` repository가 소유한다.
- production migration 실행 정책은 future NixOS module에서 설정 가능하게 하지만 실제 production apply, backup restore, legacy import, cutover는 별도 사용자 승인 전에는 수행하지 않는다.
- 현재 RPO/RTO와 production capacity 수치는 이 repository가 임의로 정하지 않는다. homelab 운영 설계와 함께 별도로 확정한다.

## First migration rationale

`0001_core_reliable_messaging.sql`은 다음 일곱 table만 만든다.

- legacy 의미를 보존하는 `users`, `groups`, `memberships`, `chatrooms`, `messages`
- durable recovery 원본인 `conversation_events`
- Redis publish 전후를 이어 주는 `outbox_events`

D1=A 사용자 결정을 따라 `conversation_events`에는 pruning, retained-floor, snapshot pointer, expiry column을 넣지 않는다. `outbox_events`는 versioned conversation intent와 membership/group control intent를 같은 typed shape로 저장하고, 이후 worker가 lease generation과 PostgreSQL clock을 사용해 claim/reclaim/CAS를 구현할 수 있는 상태만 제공한다. Redis publish나 worker runtime 자체는 이 migration의 책임이 아니다.

## Consequences

- 장점: migration order와 recovery evidence가 source control에서 재현 가능하다.
- 장점: 실패한 migration이 partial application schema를 남기지 않는지 owner test가 직접 증명한다.
- 장점: 아직 없는 table을 조기에 참조하거나 추측성 rollback으로 운영 데이터를 훼손하는 위험을 줄인다.
- 비용: 잘못된 schema도 기존 migration 수정이 아니라 새 forward-fix와 별도 검증이 필요하다.
- 비용: production rollback은 단순 down SQL이 아니라 restore 또는 forward-fix 판단을 요구하며, 그 실행은 운영 승인 절차를 따른다.

## References

- PostgreSQL 17 identity columns: <https://www.postgresql.org/docs/17/ddl-identity-columns.html>
- PostgreSQL 17 partial indexes: <https://www.postgresql.org/docs/17/indexes-partial.html>
- PostgreSQL 17 transaction blocks: <https://www.postgresql.org/docs/17/sql-begin.html>
- SQLx `Migrator`: <https://docs.rs/sqlx/0.9.0/sqlx/migrate/struct.Migrator.html>
