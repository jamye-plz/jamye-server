use std::{
    fmt::Debug,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use jamye_server::{
    adapters::{
        oauth::OsCredentialSource,
        postgres::{
            account_deletion::PostgresAccountDeletionRepository, groups::PostgresGroupsRepository,
            push::PostgresPushRepository, transactions::SqlxTransactionManager,
        },
    },
    application::{
        account_deletion::{
            ANONYMOUS_AUTHOR_NICKNAME, AccountDeletionDependencies, AccountDeletionError,
            AccountDeletionService,
        },
        groups::{
            GroupsDependencies, GroupsEndpointRateLimit, GroupsRateLimitPolicy, GroupsService,
            SystemGroupsClock,
        },
        realtime::membership_revocation::RealtimeControlIntent,
    },
    ports::{
        account_deletion::{
            AccountDeletionCommand, AccountDeletionPreparation, AccountDeletionReport,
            AccountDeletionRepository, AccountDeletionRepositoryError,
            AccountDeletionRepositoryFuture,
        },
        groups::{
            CreateGroupCommand, CreateInviteCommand, GetGroupQuery, GroupActorCommand, GroupPage,
            GroupRecord, GroupsRepository, GroupsRepositoryFuture, InviteJoinRecord, InviteRecord,
            ListGroupsQuery, ListMembersQuery, MemberPage, RedeemInviteCommand,
            RemoveMemberCommand, RenameGroupCommand, SetMemberRoleCommand,
        },
        push::{
            FenceGroupPushCommand, FenceMembershipPushCommand, PushPrivacyFence,
            PushPrivacyFenceFuture,
        },
        rate_limit::{RateLimitFuture, RateLimitOutcome, RateLimitRequest, RateLimiter},
        transactions::{
            BoxTransactionHandle, TransactionFuture, TransactionHandle, TransactionManager,
        },
    },
};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::{
    TestResult,
    postgres_support::TestDatabase,
    support::{require, require_eq, test_error},
};

const UNBOUND_OBJECT_KEYS: [&str; 3] = [
    "account-delete/confirmed-unbound",
    "account-delete/expired-unbound",
    "account-delete/pending-unbound",
];

#[tokio::test]
async fn live_owned_group_blocks_with_one_rollback_and_byte_equivalent_zero_mutation() -> TestResult
{
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let target_id = fixture_id(1);
        insert_user(&pool, target_id, "deletion target").await?;
        insert_group_with_membership(
            &pool,
            fixture_id(10),
            fixture_id(11),
            target_id,
            fixture_id(12),
            target_id,
            "owner",
        )
        .await?;
        seed_private_rows(&pool, target_id, fixture_id(10), fixture_id(12)).await?;

        let before = database_snapshot(&pool).await?;
        let harness = service_harness(&pool, RepositoryMode::Normal)?;
        let deletion = harness
            .service
            .delete_account(AccountDeletionCommand { user_id: target_id })
            .await;
        require_eq(
            deletion,
            Err(AccountDeletionError::GroupOwnershipTransferRequired),
            "a live target-owned group did not produce the stable ownership-transfer conflict",
        )?;
        require_eq(
            harness.transactions.events(),
            vec![TransactionEventKind::Begin, TransactionEventKind::Rollback],
            "D5 conflict did not use exactly one begin and one rollback",
        )?;
        require_eq(
            database_snapshot(&pool).await?,
            before,
            "D5 conflict changed PostgreSQL rows or bytes",
        )
    }
    .await;

    finish_database_test(database, pool, result).await
}

#[tokio::test]
async fn d10_reassigns_retained_content_removes_private_state_and_enqueues_unique_cleanup()
-> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let fixture = seed_full_deletion_fixture(&pool).await?;
        let unrelated_before =
            unrelated_control_snapshot(&pool, fixture.unrelated_outbox_id).await?;
        let harness = service_harness(&pool, RepositoryMode::Normal)?;

        let report = harness
            .service
            .delete_account(AccountDeletionCommand {
                user_id: fixture.target_id,
            })
            .await;
        require_eq(
            report,
            Ok(AccountDeletionReport {
                memberships_removed: 2,
                cleanup_intents_enqueued: 3,
            }),
            "D10 deletion did not report the committed transition",
        )?;
        require_eq(
            harness.transactions.events(),
            vec![TransactionEventKind::Begin, TransactionEventKind::Commit],
            "D10 deletion did not use exactly one begin and one commit",
        )?;

        let tombstone = tombstone_projection(&pool).await?;
        require_eq(
            tombstone.1,
            ANONYMOUS_AUTHOR_NICKNAME.to_owned(),
            "tombstone nickname differs from the fixed anonymous projection",
        )?;
        require(
            tombstone.2.is_none(),
            "tombstone unexpectedly retained an avatar projection",
        )?;
        require(
            tombstone.0 != fixture.target_id,
            "deletion reused the authenticating account as its tombstone",
        )?;

        require_no_private_references(&pool, fixture.target_id).await?;
        require_retained_content(&pool, &fixture, tombstone.0).await?;
        require_cleanup_intents(&pool).await?;
        require_payloads_scrubbed(&pool, fixture.target_id).await?;
        require_eq(
            unrelated_control_snapshot(&pool, fixture.unrelated_outbox_id).await?,
            unrelated_before,
            "deletion changed unrelated control state",
        )
    }
    .await;

    finish_database_test(database, pool, result).await
}

#[tokio::test]
async fn source_authored_occurrence_is_fenced_without_deleting_live_recipient_state() -> TestResult
{
    run_source_authored_occurrence_scenario().await
}

// The source and unrelated-control deliveries share an installation but must
// reach different post-delete states. Keep their deterministic fixture private.
#[derive(Clone, Copy)]
struct SourceAuthoredOccurrenceFixture {
    target_id: Uuid,
    recipient_id: Uuid,
    group_id: Uuid,
    chatroom_id: Uuid,
    topic_id: Uuid,
    message_id: Uuid,
    event_id: Uuid,
    notification_id: Uuid,
    installation_id: Uuid,
    occurrence_id: Uuid,
    control_message_id: Uuid,
    control_event_id: Uuid,
    control_notification_id: Uuid,
    control_occurrence_id: Uuid,
    source_cursor: i64,
}

#[derive(Clone, Copy)]
struct DeliveryOccurrenceSeed {
    occurrence_id: Uuid,
    notification_id: Uuid,
    event_id: Uuid,
    message_id: Uuid,
    recipient_id: Uuid,
    installation_id: Uuid,
    chatroom_id: Uuid,
}

impl SourceAuthoredOccurrenceFixture {
    fn deterministic() -> Self {
        Self {
            target_id: fixture_id(1),
            recipient_id: fixture_id(2),
            group_id: fixture_id(80),
            chatroom_id: fixture_id(81),
            topic_id: fixture_id(82),
            message_id: fixture_id(83),
            event_id: fixture_id(84),
            notification_id: fixture_id(85),
            installation_id: fixture_id(86),
            occurrence_id: fixture_id(87),
            control_message_id: fixture_id(94),
            control_event_id: fixture_id(95),
            control_notification_id: fixture_id(96),
            control_occurrence_id: fixture_id(97),
            source_cursor: 0,
        }
    }

    fn source_delivery_seed(self) -> DeliveryOccurrenceSeed {
        DeliveryOccurrenceSeed {
            occurrence_id: self.occurrence_id,
            notification_id: self.notification_id,
            event_id: self.event_id,
            message_id: self.message_id,
            recipient_id: self.recipient_id,
            installation_id: self.installation_id,
            chatroom_id: self.chatroom_id,
        }
    }

    fn control_delivery_seed(self) -> DeliveryOccurrenceSeed {
        DeliveryOccurrenceSeed {
            occurrence_id: self.control_occurrence_id,
            notification_id: self.control_notification_id,
            event_id: self.control_event_id,
            message_id: self.control_message_id,
            recipient_id: self.recipient_id,
            installation_id: self.installation_id,
            chatroom_id: self.chatroom_id,
        }
    }
}

async fn seed_source_authored_occurrence_fixture(
    pool: &PgPool,
) -> TestResult<SourceAuthoredOccurrenceFixture> {
    let mut fixture = SourceAuthoredOccurrenceFixture::deterministic();
    seed_source_authored_base(pool, fixture).await?;
    fixture.source_cursor = seed_source_authored_message_event(pool, fixture).await?;
    seed_source_authored_notification(pool, fixture).await?;
    seed_live_recipient_installation(pool, fixture).await?;
    insert_delivery_occurrence(pool, fixture.source_delivery_seed()).await?;
    let control_cursor = seed_unrelated_control_message_event(pool, fixture).await?;
    seed_unrelated_control_notification(pool, fixture, control_cursor).await?;
    insert_delivery_occurrence(pool, fixture.control_delivery_seed()).await?;
    Ok(fixture)
}

async fn seed_source_authored_base(
    pool: &PgPool,
    fixture: SourceAuthoredOccurrenceFixture,
) -> TestResult {
    insert_user(pool, fixture.target_id, "deleted source author").await?;
    insert_user(pool, fixture.recipient_id, "live notification recipient").await?;
    insert_group_with_membership(
        pool,
        fixture.group_id,
        fixture_id(88),
        fixture.recipient_id,
        fixture.chatroom_id,
        fixture.recipient_id,
        "owner",
    )
    .await?;
    sqlx::query(
        "INSERT INTO topics \
             (id, group_id, author_id, idempotency_key, request_fingerprint, title, body) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(fixture.topic_id)
    .bind(fixture.group_id)
    .bind(fixture.recipient_id)
    .bind(fixture_id(820))
    .bind("b".repeat(64))
    .bind("retained notification topic")
    .bind("retained notification topic body")
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_source_authored_message_event(
    pool: &PgPool,
    fixture: SourceAuthoredOccurrenceFixture,
) -> TestResult<i64> {
    sqlx::query(
        "INSERT INTO messages \
             (id, chatroom_id, sender_id, client_msg_id, body, type) \
         VALUES ($1, $2, $3, $4, $5, 'user')",
    )
    .bind(fixture.message_id)
    .bind(fixture.chatroom_id)
    .bind(fixture.target_id)
    .bind(fixture_id(830))
    .bind("retained cross-recipient message")
    .execute(pool)
    .await?;
    Ok(sqlx::query_scalar::<_, i64>(
        "INSERT INTO conversation_events \
             (id, conversation_id, event_type, event_version, payload) \
         VALUES ($1, $2, 'message.created', 1, $3) RETURNING cursor",
    )
    .bind(fixture.event_id)
    .bind(fixture.chatroom_id)
    .bind(json!({
        "id": fixture.message_id,
        "chatroom_id": fixture.chatroom_id,
        "sender_id": fixture.target_id,
        "body": "retained cross-recipient message",
    }))
    .fetch_one(pool)
    .await?)
}

async fn seed_source_authored_notification(
    pool: &PgPool,
    fixture: SourceAuthoredOccurrenceFixture,
) -> TestResult {
    sqlx::query(
        "INSERT INTO notifications \
             (id, user_id, topic_id, conversation_id, source_cursor, type, payload, dedup_key) \
         VALUES ($1, $2, $3, $4, $5, 'chat_unread', $6, $7)",
    )
    .bind(fixture.notification_id)
    .bind(fixture.recipient_id)
    .bind(fixture.topic_id)
    .bind(fixture.chatroom_id)
    .bind(fixture.source_cursor)
    .bind(json!({"sender_display_name": "deleted source author"}))
    .bind("account-delete:retained-recipient-notification")
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_live_recipient_installation(
    pool: &PgPool,
    fixture: SourceAuthoredOccurrenceFixture,
) -> TestResult {
    sqlx::query(
        "INSERT INTO push_installations \
             (id, user_id, installation_id, platform, provider, token, environment) \
         VALUES ($1, $2, $3, 'ios', 'expo', $4, 'development')",
    )
    .bind(fixture.installation_id)
    .bind(fixture.recipient_id)
    .bind("account-delete-live-recipient-device")
    .bind("ExponentPushToken[account-delete-live-recipient]")
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_delivery_occurrence(pool: &PgPool, seed: DeliveryOccurrenceSeed) -> TestResult {
    sqlx::query(
        "INSERT INTO push_delivery_intents \
             (id, notification_id, source_event_id, source_message_id, recipient_user_id, \
              push_installation_id, installation_owner_epoch, \
              message_preview_enabled_snapshot, payload, status) \
         VALUES ($1, $2, $3, $4, $5, $6, 1, false, $7, 'pending')",
    )
    .bind(seed.occurrence_id)
    .bind(seed.notification_id)
    .bind(seed.event_id)
    .bind(seed.message_id)
    .bind(seed.recipient_id)
    .bind(seed.installation_id)
    .bind(json!({
        "type": "chat_unread",
        "notification_id": seed.notification_id,
        "conversation_id": seed.chatroom_id,
        "message_id": seed.message_id,
    }))
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_unrelated_control_message_event(
    pool: &PgPool,
    fixture: SourceAuthoredOccurrenceFixture,
) -> TestResult<i64> {
    sqlx::query(
        "INSERT INTO messages \
             (id, chatroom_id, sender_id, client_msg_id, body, type) \
         VALUES ($1, $2, $3, $4, $5, 'user')",
    )
    .bind(fixture.control_message_id)
    .bind(fixture.chatroom_id)
    .bind(fixture.recipient_id)
    .bind(fixture_id(940))
    .bind("unrelated live-recipient message")
    .execute(pool)
    .await?;
    Ok(sqlx::query_scalar::<_, i64>(
        "INSERT INTO conversation_events \
             (id, conversation_id, event_type, event_version, payload) \
         VALUES ($1, $2, 'message.created', 1, $3) RETURNING cursor",
    )
    .bind(fixture.control_event_id)
    .bind(fixture.chatroom_id)
    .bind(json!({
        "id": fixture.control_message_id,
        "chatroom_id": fixture.chatroom_id,
        "sender_id": fixture.recipient_id,
        "body": "unrelated live-recipient message",
    }))
    .fetch_one(pool)
    .await?)
}

async fn seed_unrelated_control_notification(
    pool: &PgPool,
    fixture: SourceAuthoredOccurrenceFixture,
    source_cursor: i64,
) -> TestResult {
    sqlx::query(
        "INSERT INTO notifications \
             (id, user_id, topic_id, conversation_id, source_cursor, type, payload, dedup_key) \
         VALUES ($1, $2, $3, $4, $5, 'chat_unread', $6, $7)",
    )
    .bind(fixture.control_notification_id)
    .bind(fixture.recipient_id)
    .bind(fixture.topic_id)
    .bind(fixture.chatroom_id)
    .bind(source_cursor)
    .bind(json!({"sender_display_name": "live notification recipient"}))
    .bind("account-delete:unrelated-recipient-notification")
    .execute(pool)
    .await?;
    Ok(())
}

async fn run_source_authored_occurrence_scenario() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let fixture = seed_source_authored_occurrence_fixture(&pool).await?;
        delete_and_assert_source_authored_occurrence(&pool, fixture).await
    }
    .await;

    finish_database_test(database, pool, result).await
}

async fn delete_and_assert_source_authored_occurrence(
    pool: &PgPool,
    fixture: SourceAuthoredOccurrenceFixture,
) -> TestResult {
    let harness = service_harness(pool, RepositoryMode::Normal)?;
    require_eq(
        harness
            .service
            .delete_account(AccountDeletionCommand {
                user_id: fixture.target_id,
            })
            .await,
        Ok(AccountDeletionReport {
            memberships_removed: 0,
            cleanup_intents_enqueued: 0,
        }),
        "source-author deletion did not commit",
    )?;
    let tombstone = tombstone_projection(pool).await?;
    assert_source_authored_message_and_event(
        pool,
        fixture.message_id,
        fixture.event_id,
        tombstone.0,
    )
    .await?;
    assert_live_recipient_notification(
        pool,
        fixture.notification_id,
        fixture.recipient_id,
        fixture.topic_id,
        fixture.chatroom_id,
        fixture.source_cursor,
    )
    .await?;
    assert_live_recipient_installation(pool, fixture.installation_id, fixture.recipient_id).await?;
    assert_source_authored_occurrence_is_terminal(pool, fixture.occurrence_id).await?;
    assert_unrelated_occurrence_is_unchanged(
        pool,
        fixture.control_occurrence_id,
        fixture.control_notification_id,
        fixture.control_event_id,
        fixture.installation_id,
    )
    .await
}

async fn assert_source_authored_message_and_event(
    pool: &PgPool,
    message_id: Uuid,
    event_id: Uuid,
    tombstone_user_id: Uuid,
) -> TestResult {
    require_eq(
        sqlx::query_as::<_, (Option<Uuid>, Option<String>)>(
            "SELECT sender_id, body FROM messages WHERE id = $1",
        )
        .bind(message_id)
        .fetch_one(pool)
        .await?,
        (
            Some(tombstone_user_id),
            Some("retained cross-recipient message".to_owned()),
        ),
        "source-authored message was not retained under the tombstone",
    )?;
    require_eq(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM conversation_events \
             WHERE id = $1 \
               AND payload ->> 'sender_id' = $2",
        )
        .bind(event_id)
        .bind(tombstone_user_id.to_string())
        .fetch_one(pool)
        .await?,
        1,
        "source-authored event was not retained under an anonymous projection",
    )
}

async fn assert_live_recipient_notification(
    pool: &PgPool,
    notification_id: Uuid,
    recipient_id: Uuid,
    topic_id: Uuid,
    chatroom_id: Uuid,
    source_cursor: i64,
) -> TestResult {
    require_eq(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM notifications \
             WHERE id = $1 \
               AND user_id = $2 \
               AND topic_id = $3 \
               AND conversation_id = $4 \
               AND source_cursor = $5 \
               AND payload ->> 'sender_display_name' = $6",
        )
        .bind(notification_id)
        .bind(recipient_id)
        .bind(topic_id)
        .bind(chatroom_id)
        .bind(source_cursor)
        .bind(ANONYMOUS_AUTHOR_NICKNAME)
        .fetch_one(pool)
        .await?,
        1,
        "the live recipient's notification was deleted or retained its private profile",
    )
}

async fn assert_live_recipient_installation(
    pool: &PgPool,
    installation_id: Uuid,
    recipient_id: Uuid,
) -> TestResult {
    require_eq(
        sqlx::query_as::<_, (i64, Option<time::OffsetDateTime>)>(
            "SELECT owner_epoch, disabled_at FROM push_installations \
             WHERE id = $1 AND user_id = $2",
        )
        .bind(installation_id)
        .bind(recipient_id)
        .fetch_optional(pool)
        .await?,
        Some((1, None)),
        "the live recipient's installation was deleted, disabled, or rebound",
    )
}

async fn assert_source_authored_occurrence_is_terminal(
    pool: &PgPool,
    occurrence_id: Uuid,
) -> TestResult {
    require_eq(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM push_delivery_intents \
             WHERE id = $1 AND status IN ('pending', 'claimed', 'retryable')",
        )
        .bind(occurrence_id)
        .fetch_one(pool)
        .await?,
        0,
        "a source-authored pending occurrence survived and can be reclaimed",
    )?;
    if let Some(status) =
        sqlx::query_scalar::<_, String>("SELECT status FROM push_delivery_intents WHERE id = $1")
            .bind(occurrence_id)
            .fetch_optional(pool)
            .await?
    {
        require(
            matches!(status.as_str(), "failed" | "dead_letter"),
            "a retained source-authored occurrence was not terminalized",
        )?;
    }
    Ok(())
}

async fn assert_unrelated_occurrence_is_unchanged(
    pool: &PgPool,
    occurrence_id: Uuid,
    notification_id: Uuid,
    event_id: Uuid,
    installation_id: Uuid,
) -> TestResult {
    require_eq(
        sqlx::query_as::<_, (Uuid, Uuid, Uuid, i64, String)>(
            "SELECT notification_id, source_event_id, push_installation_id, \
                    installation_owner_epoch, status \
             FROM push_delivery_intents WHERE id = $1",
        )
        .bind(occurrence_id)
        .fetch_one(pool)
        .await?,
        (
            notification_id,
            event_id,
            installation_id,
            1,
            "pending".to_owned(),
        ),
        "account deletion changed an unrelated live-recipient occurrence",
    )
}

#[tokio::test]
async fn target_routed_membership_revocation_control_intent_remains_exact_and_decodable()
-> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let target_id = fixture_id(1);
        let group_owner_id = fixture_id(2);
        let group_id = fixture_id(90);
        let control_id = fixture_id(91);
        insert_user(&pool, target_id, "deleted control target").await?;
        insert_user(&pool, group_owner_id, "retained control owner").await?;
        insert_group_with_membership(
            &pool,
            group_id,
            fixture_id(92),
            group_owner_id,
            fixture_id(93),
            group_owner_id,
            "owner",
        )
        .await?;

        let control = RealtimeControlIntent::MembershipRevoked {
            version: 1,
            control_id,
            group_id,
            user_id: target_id,
        };
        sqlx::query(
            "INSERT INTO outbox_events \
                 (id, intent_type, event_type, event_version, aggregate_type, aggregate_id, payload) \
             VALUES ($1, 'control', 'membership.revoked', 1, 'membership', $2, $3)",
        )
        .bind(control_id)
        .bind(target_id)
        .bind(serde_json::to_value(&control)?)
        .execute(&pool)
        .await?;
        let before = control_protocol_snapshot(&pool, control_id).await?;

        let harness = service_harness(&pool, RepositoryMode::Normal)?;
        require_eq(
            harness
                .service
                .delete_account(AccountDeletionCommand { user_id: target_id })
                .await,
            Ok(AccountDeletionReport {
                memberships_removed: 0,
                cleanup_intents_enqueued: 0,
            }),
            "control-target account deletion did not commit",
        )?;

        let payload = sqlx::query_scalar::<_, Value>(
            "SELECT payload FROM outbox_events WHERE id = $1",
        )
        .bind(control_id)
        .fetch_one(&pool)
        .await?;
        let decoded = serde_json::from_value::<RealtimeControlIntent>(payload)?;
        require_eq(
            decoded,
            control,
            "membership.revoked routing identity changed after account deletion",
        )?;
        require_eq(
            control_protocol_snapshot(&pool, control_id).await?,
            before,
            "membership.revoked protocol header or payload changed",
        )
    }
    .await;

    finish_database_test(database, pool, result).await
}

#[tokio::test]
async fn memberships_are_removed_and_fenced_in_membership_id_order_on_one_handle() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let target_id = fixture_id(1);
        let owner_a = fixture_id(2);
        let owner_b = fixture_id(3);
        insert_user(&pool, target_id, "ordered target").await?;
        insert_user(&pool, owner_a, "owner a").await?;
        insert_user(&pool, owner_b, "owner b").await?;

        let group_a = fixture_id(100);
        let group_b = fixture_id(200);
        let membership_a = fixture_id(201);
        let membership_b = fixture_id(101);
        insert_group_with_membership(
            &pool,
            group_a,
            fixture_id(110),
            owner_a,
            fixture_id(111),
            target_id,
            "member",
        )
        .await?;
        replace_membership_id(&pool, group_a, target_id, membership_a).await?;
        insert_group_with_membership(
            &pool,
            group_b,
            fixture_id(210),
            owner_b,
            fixture_id(211),
            target_id,
            "member",
        )
        .await?;
        replace_membership_id(&pool, group_b, target_id, membership_b).await?;

        let harness = service_harness(&pool, RepositoryMode::Normal)?;
        let report = harness
            .service
            .delete_account(AccountDeletionCommand { user_id: target_id })
            .await;
        require_eq(
            report,
            Ok(AccountDeletionReport {
                memberships_removed: 2,
                cleanup_intents_enqueued: 0,
            }),
            "ordered deletion did not complete",
        )?;

        let calls = harness.membership_calls.snapshot();
        require_eq(
            calls
                .iter()
                .map(|call| (call.boundary, call.group_id))
                .collect::<Vec<_>>(),
            vec![
                (MembershipBoundary::Groups, group_b),
                (MembershipBoundary::Push, group_b),
                (MembershipBoundary::Groups, group_a),
                (MembershipBoundary::Push, group_a),
            ],
            "Task-6 removals and Task-9 fences did not interleave in membership-id order",
        )?;
        let transaction_handle = harness
            .transactions
            .single_handle()
            .ok_or_else(|| test_error("transaction recorder did not retain one opaque handle"))?;
        require(
            calls.len() == 4
                && calls
                    .iter()
                    .all(|call| call.handle_id == transaction_handle),
            "Task-6 and Task-9 did not receive the same caller-owned opaque handle",
        )?;
        require_eq(
            harness.transactions.events(),
            vec![TransactionEventKind::Begin, TransactionEventKind::Commit],
            "ordered transition opened or completed more than one transaction",
        )
    }
    .await;

    finish_database_test(database, pool, result).await
}

#[tokio::test]
async fn finalize_failure_rolls_back_group_fences_tombstone_and_cleanup_intents() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let fixture = seed_full_deletion_fixture(&pool).await?;
        let before = database_snapshot(&pool).await?;
        let finalized = Arc::new(AtomicBool::new(false));
        let harness = service_harness(&pool, RepositoryMode::FailAfterFinalize(finalized.clone()))?;

        let deletion = harness
            .service
            .delete_account(AccountDeletionCommand {
                user_id: fixture.target_id,
            })
            .await;
        require_eq(
            deletion,
            Err(AccountDeletionError::DatabaseUnavailable),
            "injected post-finalize failure did not map to DatabaseUnavailable",
        )?;
        require(
            finalized.load(Ordering::SeqCst),
            "failure decorator did not observe a successful real finalize",
        )?;
        require_eq(
            harness.transactions.events(),
            vec![TransactionEventKind::Begin, TransactionEventKind::Rollback],
            "post-finalize failure did not use exactly one begin and one rollback",
        )?;
        require_eq(
            database_snapshot(&pool).await?,
            before,
            "post-finalize rollback leaked a membership, fence, tombstone, or cleanup mutation",
        )
    }
    .await;

    finish_database_test(database, pool, result).await
}

#[tokio::test]
async fn archived_owned_group_is_reassigned_to_tombstone_without_live_d5_conflict() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let target_id = fixture_id(1);
        let group_id = fixture_id(20);
        let membership_id = fixture_id(21);
        insert_user(&pool, target_id, "archived owner").await?;
        insert_group_with_membership(
            &pool,
            group_id,
            membership_id,
            target_id,
            fixture_id(22),
            target_id,
            "owner",
        )
        .await?;
        sqlx::query("UPDATE groups SET deleted_at = clock_timestamp() WHERE id = $1")
            .bind(group_id)
            .execute(&pool)
            .await?;

        let harness = service_harness(&pool, RepositoryMode::Normal)?;
        let report = harness
            .service
            .delete_account(AccountDeletionCommand { user_id: target_id })
            .await;
        require_eq(
            report,
            Ok(AccountDeletionReport {
                memberships_removed: 1,
                cleanup_intents_enqueued: 0,
            }),
            "soft-deleted ownership was incorrectly treated as a live D5 conflict",
        )?;
        let tombstone = tombstone_projection(&pool).await?;
        let archived = sqlx::query_as::<_, (Uuid, bool)>(
            "SELECT owner_id, deleted_at IS NOT NULL FROM groups WHERE id = $1",
        )
        .bind(group_id)
        .fetch_one(&pool)
        .await?;
        require_eq(
            archived,
            (tombstone.0, true),
            "archived group was not retained under tombstone ownership",
        )?;
        require_eq(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM memberships WHERE id = $1 OR user_id = $2",
            )
            .bind(membership_id)
            .bind(target_id)
            .fetch_one(&pool)
            .await?,
            0,
            "archived owner membership survived deletion",
        )?;
        require_eq(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users WHERE id = $1")
                .bind(target_id)
                .fetch_one(&pool)
                .await?,
            0,
            "archived owner account survived deletion",
        )
    }
    .await;

    finish_database_test(database, pool, result).await
}

#[derive(Clone)]
enum RepositoryMode {
    Normal,
    FailAfterFinalize(Arc<AtomicBool>),
}

struct ServiceHarness {
    service: AccountDeletionService,
    transactions: Arc<RecordingTransactionManager>,
    membership_calls: MembershipCallLog,
}

fn service_harness(pool: &PgPool, repository_mode: RepositoryMode) -> TestResult<ServiceHarness> {
    let transactions = Arc::new(RecordingTransactionManager::new(
        SqlxTransactionManager::new(pool.clone()),
    ));
    let membership_calls = MembershipCallLog::default();
    let groups_repository = Arc::new(RecordingGroupsRepository {
        inner: PostgresGroupsRepository::new(pool.clone()),
        calls: membership_calls.clone(),
    });
    let groups = Arc::new(GroupsService::new(
        GroupsDependencies {
            transactions: transactions.clone(),
            repository: groups_repository,
            rate_limiter: Arc::new(AllowRateLimiter),
            credentials: Arc::new(OsCredentialSource),
            clock: Arc::new(SystemGroupsClock),
        },
        GroupsRateLimitPolicy {
            invite_issue: GroupsEndpointRateLimit {
                limit: 10,
                window: Duration::from_secs(60),
            },
            invite_redeem: GroupsEndpointRateLimit {
                limit: 20,
                window: Duration::from_secs(60),
            },
        },
    )?);
    let push_privacy_fence = Arc::new(RecordingPushPrivacyFence {
        inner: PostgresPushRepository::new(pool.clone()),
        calls: membership_calls.clone(),
    });
    let postgres_repository = PostgresAccountDeletionRepository::new(pool.clone());
    let repository: Arc<dyn AccountDeletionRepository> = match repository_mode {
        RepositoryMode::Normal => Arc::new(postgres_repository),
        RepositoryMode::FailAfterFinalize(finalized) => Arc::new(FailAfterFinalizeRepository {
            inner: postgres_repository,
            finalized,
        }),
    };
    let service = AccountDeletionService::new(AccountDeletionDependencies {
        transactions: transactions.clone(),
        groups,
        push_privacy_fence,
        repository,
    });
    Ok(ServiceHarness {
        service,
        transactions,
        membership_calls,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransactionEventKind {
    Begin,
    Commit,
    Rollback,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TransactionEvent {
    kind: TransactionEventKind,
    handle_id: usize,
}

struct RecordingTransactionManager {
    inner: SqlxTransactionManager,
    events: Mutex<Vec<TransactionEvent>>,
}

impl RecordingTransactionManager {
    fn new(inner: SqlxTransactionManager) -> Self {
        Self {
            inner,
            events: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, kind: TransactionEventKind, handle_id: usize) {
        mutex_guard(&self.events).push(TransactionEvent { kind, handle_id });
    }

    fn events(&self) -> Vec<TransactionEventKind> {
        mutex_guard(&self.events)
            .iter()
            .map(|event| event.kind)
            .collect()
    }

    fn single_handle(&self) -> Option<usize> {
        let events = mutex_guard(&self.events);
        let first = events.first()?.handle_id;
        events
            .iter()
            .all(|event| event.handle_id == first)
            .then_some(first)
    }
}

impl TransactionManager for RecordingTransactionManager {
    fn begin(&self) -> TransactionFuture<'_, BoxTransactionHandle> {
        Box::pin(async move {
            let mut handle = self.inner.begin().await?;
            let handle_id = transaction_handle_id(handle.as_mut());
            self.record(TransactionEventKind::Begin, handle_id);
            Ok(handle)
        })
    }

    fn commit<'a>(&'a self, mut handle: BoxTransactionHandle) -> TransactionFuture<'a, ()> {
        Box::pin(async move {
            self.record(
                TransactionEventKind::Commit,
                transaction_handle_id(handle.as_mut()),
            );
            self.inner.commit(handle).await
        })
    }

    fn rollback<'a>(&'a self, mut handle: BoxTransactionHandle) -> TransactionFuture<'a, ()> {
        Box::pin(async move {
            self.record(
                TransactionEventKind::Rollback,
                transaction_handle_id(handle.as_mut()),
            );
            self.inner.rollback(handle).await
        })
    }
}

fn transaction_handle_id(transaction: &mut dyn TransactionHandle) -> usize {
    let pointer = transaction as *mut dyn TransactionHandle;
    pointer as *mut () as usize
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MembershipBoundary {
    Groups,
    Push,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MembershipCall {
    boundary: MembershipBoundary,
    group_id: Uuid,
    handle_id: usize,
}

#[derive(Clone, Default)]
struct MembershipCallLog(Arc<Mutex<Vec<MembershipCall>>>);

impl MembershipCallLog {
    fn record(
        &self,
        boundary: MembershipBoundary,
        group_id: Uuid,
        transaction: &mut dyn TransactionHandle,
    ) {
        mutex_guard(&self.0).push(MembershipCall {
            boundary,
            group_id,
            handle_id: transaction_handle_id(transaction),
        });
    }

    fn snapshot(&self) -> Vec<MembershipCall> {
        mutex_guard(&self.0).clone()
    }
}

struct RecordingGroupsRepository {
    inner: PostgresGroupsRepository,
    calls: MembershipCallLog,
}

impl GroupsRepository for RecordingGroupsRepository {
    fn create_group<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a CreateGroupCommand,
    ) -> GroupsRepositoryFuture<'a, GroupRecord> {
        self.inner.create_group(transaction, command)
    }

    fn list_groups(&self, query: ListGroupsQuery) -> GroupsRepositoryFuture<'_, GroupPage> {
        self.inner.list_groups(query)
    }

    fn get_group(&self, query: GetGroupQuery) -> GroupsRepositoryFuture<'_, GroupRecord> {
        self.inner.get_group(query)
    }

    fn list_members(&self, query: ListMembersQuery) -> GroupsRepositoryFuture<'_, MemberPage> {
        self.inner.list_members(query)
    }

    fn rename_group<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a RenameGroupCommand,
    ) -> GroupsRepositoryFuture<'a, GroupRecord> {
        self.inner.rename_group(transaction, command)
    }

    fn delete_group<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a GroupActorCommand,
    ) -> GroupsRepositoryFuture<'a, ()> {
        self.inner.delete_group(transaction, command)
    }

    fn remove_member<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a RemoveMemberCommand,
    ) -> GroupsRepositoryFuture<'a, ()> {
        self.calls
            .record(MembershipBoundary::Groups, command.group_id, transaction);
        self.inner.remove_member(transaction, command)
    }

    fn set_member_role<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a SetMemberRoleCommand,
    ) -> GroupsRepositoryFuture<'a, ()> {
        self.inner.set_member_role(transaction, command)
    }

    fn create_invite<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a CreateInviteCommand,
    ) -> GroupsRepositoryFuture<'a, InviteRecord> {
        self.inner.create_invite(transaction, command)
    }

    fn redeem_invite<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a RedeemInviteCommand,
    ) -> GroupsRepositoryFuture<'a, InviteJoinRecord> {
        self.inner.redeem_invite(transaction, command)
    }
}

struct RecordingPushPrivacyFence {
    inner: PostgresPushRepository,
    calls: MembershipCallLog,
}

impl PushPrivacyFence for RecordingPushPrivacyFence {
    fn fence_membership_revocation<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a FenceMembershipPushCommand,
    ) -> PushPrivacyFenceFuture<'a> {
        self.calls
            .record(MembershipBoundary::Push, command.group_id, transaction);
        self.inner.fence_membership_revocation(transaction, command)
    }

    fn fence_group_deletion<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a FenceGroupPushCommand,
    ) -> PushPrivacyFenceFuture<'a> {
        self.inner.fence_group_deletion(transaction, command)
    }
}

struct FailAfterFinalizeRepository {
    inner: PostgresAccountDeletionRepository,
    finalized: Arc<AtomicBool>,
}

impl AccountDeletionRepository for FailAfterFinalizeRepository {
    fn prepare_deletion<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        user_id: Uuid,
    ) -> AccountDeletionRepositoryFuture<'a, AccountDeletionPreparation> {
        self.inner.prepare_deletion(transaction, user_id)
    }

    fn finalize_deletion<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        user_id: Uuid,
    ) -> AccountDeletionRepositoryFuture<'a, AccountDeletionReport> {
        Box::pin(async move {
            let _ = self.inner.finalize_deletion(transaction, user_id).await?;
            self.finalized.store(true, Ordering::SeqCst);
            Err(AccountDeletionRepositoryError::Unavailable)
        })
    }
}

#[derive(Clone, Copy)]
struct AllowRateLimiter;

impl RateLimiter for AllowRateLimiter {
    fn check<'a>(&'a self, _request: &'a RateLimitRequest) -> RateLimitFuture<'a> {
        Box::pin(async { Ok(RateLimitOutcome::Allowed) })
    }
}

#[derive(Debug, Eq, PartialEq)]
struct DatabaseSnapshot(String);

struct FullDeletionFixture {
    target_id: Uuid,
    message_id: Uuid,
    topic_id: Uuid,
    event_id: Uuid,
    conversation_outbox_id: Uuid,
    bound_upload_id: Uuid,
    bound_object_key: String,
    unrelated_outbox_id: Uuid,
}

fn fixture_id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

async fn insert_user(pool: &PgPool, user_id: Uuid, nickname: &str) -> TestResult {
    sqlx::query("INSERT INTO users (id, nickname, avatar_url) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(nickname)
        .bind(format!("https://private.invalid/{user_id}"))
        .execute(pool)
        .await?;
    Ok(())
}

async fn insert_group_with_membership(
    pool: &PgPool,
    group_id: Uuid,
    membership_id: Uuid,
    owner_id: Uuid,
    chatroom_id: Uuid,
    member_id: Uuid,
    member_role: &str,
) -> TestResult {
    sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, $2, $3)")
        .bind(group_id)
        .bind(format!("account-deletion-{group_id}"))
        .bind(owner_id)
        .execute(pool)
        .await?;

    if owner_id == member_id {
        sqlx::query(
            "INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, 'owner')",
        )
        .bind(membership_id)
        .bind(group_id)
        .bind(owner_id)
        .execute(pool)
        .await?;
    } else {
        let owner_membership_id = fixture_id(group_id.as_u128() + 10_000);
        sqlx::query(
            "INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, 'owner')",
        )
        .bind(owner_membership_id)
        .bind(group_id)
        .bind(owner_id)
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, $4)",
        )
        .bind(membership_id)
        .bind(group_id)
        .bind(member_id)
        .bind(member_role)
        .execute(pool)
        .await?;
    }

    sqlx::query(
        "INSERT INTO chatrooms (id, group_id, type, topic_id) VALUES ($1, $2, 'main', NULL)",
    )
    .bind(chatroom_id)
    .bind(group_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_private_rows(
    pool: &PgPool,
    target_id: Uuid,
    group_id: Uuid,
    chatroom_id: Uuid,
) -> TestResult {
    sqlx::query(
        "INSERT INTO auth_identities (id, user_id, provider, provider_id) \
         VALUES ($1, $2, 'google', $3)",
    )
    .bind(fixture_id(9_001))
    .bind(target_id)
    .bind(format!("account-deletion-{target_id}"))
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO refresh_sessions \
             (id, user_id, family_id, token_hash, expires_at) \
         VALUES ($1, $2, $3, $4, clock_timestamp() + INTERVAL '1 day')",
    )
    .bind(fixture_id(9_002))
    .bind(target_id)
    .bind(fixture_id(9_003))
    .bind(vec![9_u8; 32])
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO invites (id, group_id, code, created_by) VALUES ($1, $2, $3, $4)")
        .bind(fixture_id(9_004))
        .bind(group_id)
        .bind("account-delete-d5-invite")
        .bind(target_id)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO chatroom_reads (id, user_id, chatroom_id, last_read_cursor) \
         VALUES ($1, $2, $3, 1)",
    )
    .bind(fixture_id(9_005))
    .bind(target_id)
    .bind(chatroom_id)
    .execute(pool)
    .await?;

    let event_id = fixture_id(9_006);
    sqlx::query(
        "INSERT INTO conversation_events \
             (id, conversation_id, event_type, event_version, payload) \
         VALUES ($1, $2, 'account.deletion.fixture', 1, $3)",
    )
    .bind(event_id)
    .bind(chatroom_id)
    .bind(json!({"user_id": target_id}))
    .execute(pool)
    .await?;
    let notification_id = fixture_id(9_007);
    sqlx::query(
        "INSERT INTO notifications (id, user_id, type, payload) \
         VALUES ($1, $2, 'other', $3)",
    )
    .bind(notification_id)
    .bind(target_id)
    .bind(json!({"private": true}))
    .execute(pool)
    .await?;
    let installation_id = fixture_id(9_008);
    sqlx::query(
        "INSERT INTO push_installations \
             (id, user_id, installation_id, platform, provider, token, environment) \
         VALUES ($1, $2, $3, 'ios', 'expo', $4, 'development')",
    )
    .bind(installation_id)
    .bind(target_id)
    .bind("account-delete-d5-device")
    .bind("ExponentPushToken[account-delete-d5]")
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO push_delivery_intents \
             (id, notification_id, source_event_id, recipient_user_id, \
              push_installation_id, installation_owner_epoch, \
              message_preview_enabled_snapshot, payload) \
         VALUES ($1, $2, $3, $4, $5, 1, false, $6)",
    )
    .bind(fixture_id(9_009))
    .bind(notification_id)
    .bind(event_id)
    .bind(target_id)
    .bind(installation_id)
    .bind(json!({"private": true}))
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO media_uploads \
             (id, user_id, object_key, scope, target_id, content_type, byte_size, \
              status, expires_at) \
         VALUES ($1, $2, $3, 'chat', $4, 'image/jpeg', 32, 'pending', \
                 clock_timestamp() + INTERVAL '1 day')",
    )
    .bind(fixture_id(9_010))
    .bind(target_id)
    .bind("account-delete/d5-unbound")
    .bind(chatroom_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_full_deletion_fixture(pool: &PgPool) -> TestResult<FullDeletionFixture> {
    let target_id = fixture_id(1);
    let owner_a = fixture_id(2);
    let owner_b = fixture_id(3);
    insert_user(pool, target_id, "private target nickname").await?;
    insert_user(pool, owner_a, "retained owner a").await?;
    insert_user(pool, owner_b, "retained owner b").await?;

    let group_a = fixture_id(30);
    let group_b = fixture_id(40);
    let chatroom_a = fixture_id(32);
    let chatroom_b = fixture_id(42);
    insert_group_with_membership(
        pool,
        group_a,
        fixture_id(31),
        owner_a,
        chatroom_a,
        target_id,
        "member",
    )
    .await?;
    insert_group_with_membership(
        pool,
        group_b,
        fixture_id(41),
        owner_b,
        chatroom_b,
        target_id,
        "member",
    )
    .await?;

    let topic_id = fixture_id(60);
    sqlx::query(
        "INSERT INTO topics \
             (id, group_id, author_id, idempotency_key, request_fingerprint, title, body) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(topic_id)
    .bind(group_a)
    .bind(target_id)
    .bind(fixture_id(601))
    .bind("a".repeat(64))
    .bind("retained topic")
    .bind("retained topic body")
    .execute(pool)
    .await?;

    let message_id = fixture_id(61);
    sqlx::query(
        "INSERT INTO messages \
             (id, chatroom_id, sender_id, client_msg_id, body, type) \
         VALUES ($1, $2, $3, $4, $5, 'user')",
    )
    .bind(message_id)
    .bind(chatroom_a)
    .bind(target_id)
    .bind(fixture_id(611))
    .bind("retained message body")
    .execute(pool)
    .await?;

    let event_id = fixture_id(62);
    let source_cursor = sqlx::query_scalar::<_, i64>(
        "INSERT INTO conversation_events \
             (id, conversation_id, event_type, event_version, payload) \
         VALUES ($1, $2, 'message.created', 1, $3) RETURNING cursor",
    )
    .bind(event_id)
    .bind(chatroom_a)
    .bind(json!({
        "message_id": message_id,
        "sender_id": target_id,
        "sender_nickname": "private target nickname",
    }))
    .fetch_one(pool)
    .await?;

    let conversation_outbox_id = fixture_id(63);
    sqlx::query(
        "INSERT INTO outbox_events \
             (id, intent_type, event_type, event_version, aggregate_type, aggregate_id, \
              conversation_event_id, payload) \
         VALUES ($1, 'conversation', 'message.created', 1, 'conversation', $2, $3, $4)",
    )
    .bind(conversation_outbox_id)
    .bind(chatroom_a)
    .bind(event_id)
    .bind(json!({
        "message_id": message_id,
        "sender_id": target_id,
        "sender_nickname": "private target nickname",
    }))
    .execute(pool)
    .await?;

    let unrelated_outbox_id = fixture_id(64);
    sqlx::query(
        "INSERT INTO outbox_events \
             (id, intent_type, event_type, event_version, aggregate_type, aggregate_id, payload) \
         VALUES ($1, 'control', 'membership.retained', 1, 'membership', $2, $3)",
    )
    .bind(unrelated_outbox_id)
    .bind(fixture_id(640))
    .bind(json!({"user_id": owner_a, "unchanged": true}))
    .execute(pool)
    .await?;

    sqlx::query(
        "INSERT INTO auth_identities (id, user_id, provider, provider_id) \
         VALUES ($1, $2, 'kakao', $3)",
    )
    .bind(fixture_id(70))
    .bind(target_id)
    .bind("private-account-principal")
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO refresh_sessions \
             (id, user_id, family_id, token_hash, expires_at) \
         VALUES ($1, $2, $3, $4, clock_timestamp() + INTERVAL '1 day')",
    )
    .bind(fixture_id(71))
    .bind(target_id)
    .bind(fixture_id(710))
    .bind(vec![7_u8; 32])
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO invites (id, group_id, code, created_by) VALUES ($1, $2, $3, $4)")
        .bind(fixture_id(72))
        .bind(group_a)
        .bind("account-delete-private-invite")
        .bind(target_id)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO chatroom_reads (id, user_id, chatroom_id, last_read_cursor) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(fixture_id(73))
    .bind(target_id)
    .bind(chatroom_a)
    .bind(source_cursor)
    .execute(pool)
    .await?;

    let notification_id = fixture_id(65);
    sqlx::query(
        "INSERT INTO notifications \
             (id, user_id, topic_id, conversation_id, source_cursor, type, payload, dedup_key) \
         VALUES ($1, $2, $3, $4, $5, 'chat_unread', $6, $7)",
    )
    .bind(notification_id)
    .bind(target_id)
    .bind(topic_id)
    .bind(chatroom_a)
    .bind(source_cursor)
    .bind(json!({"sender_id": target_id, "sender_nickname": "private target nickname"}))
    .bind("account-delete:chat-unread")
    .execute(pool)
    .await?;
    let installation_id = fixture_id(66);
    sqlx::query(
        "INSERT INTO push_installations \
             (id, user_id, installation_id, platform, provider, token, environment, \
              message_preview_enabled) \
         VALUES ($1, $2, $3, 'android', 'expo', $4, 'development', true)",
    )
    .bind(installation_id)
    .bind(target_id)
    .bind("account-delete-device")
    .bind("ExponentPushToken[account-delete-private]")
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO push_delivery_intents \
             (id, notification_id, source_event_id, source_message_id, recipient_user_id, \
              push_installation_id, installation_owner_epoch, \
              message_preview_enabled_snapshot, payload, status) \
         VALUES ($1, $2, $3, $4, $5, $6, 1, true, $7, 'pending')",
    )
    .bind(fixture_id(67))
    .bind(notification_id)
    .bind(event_id)
    .bind(message_id)
    .bind(target_id)
    .bind(installation_id)
    .bind(json!({"notification_id": notification_id, "sender_id": target_id}))
    .execute(pool)
    .await?;

    let bound_upload_id = fixture_id(68);
    let bound_object_key = "account-delete/retained-bound".to_owned();
    sqlx::query(
        "INSERT INTO media_uploads \
             (id, user_id, object_key, scope, target_id, content_type, byte_size, \
              status, bound_message_id, confirmed_at, consumed_at, expires_at, created_at) \
         VALUES ($1, $2, $3, 'chat', $4, 'image/jpeg', 128, 'bound', $5, \
                 clock_timestamp() - INTERVAL '1 second', clock_timestamp(), \
                 clock_timestamp() + INTERVAL '1 day', \
                 clock_timestamp() - INTERVAL '2 seconds')",
    )
    .bind(bound_upload_id)
    .bind(target_id)
    .bind(&bound_object_key)
    .bind(chatroom_a)
    .bind(message_id)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO message_media \
             (id, message_id, media_upload_id, type, object_key, byte_size, position) \
         VALUES ($1, $2, $3, 'image', $4, 128, 0)",
    )
    .bind(fixture_id(69))
    .bind(message_id)
    .bind(bound_upload_id)
    .bind(&bound_object_key)
    .execute(pool)
    .await?;

    insert_unbound_upload(
        pool,
        fixture_id(74),
        target_id,
        chatroom_a,
        UNBOUND_OBJECT_KEYS[2],
        "pending",
    )
    .await?;
    insert_unbound_upload(
        pool,
        fixture_id(75),
        target_id,
        chatroom_a,
        UNBOUND_OBJECT_KEYS[0],
        "confirmed",
    )
    .await?;
    insert_unbound_upload(
        pool,
        fixture_id(76),
        target_id,
        chatroom_a,
        UNBOUND_OBJECT_KEYS[1],
        "expired",
    )
    .await?;

    Ok(FullDeletionFixture {
        target_id,
        message_id,
        topic_id,
        event_id,
        conversation_outbox_id,
        bound_upload_id,
        bound_object_key,
        unrelated_outbox_id,
    })
}

async fn insert_unbound_upload(
    pool: &PgPool,
    upload_id: Uuid,
    user_id: Uuid,
    target_id: Uuid,
    object_key: &str,
    status: &str,
) -> TestResult {
    let confirmed_at = (status == "confirmed").then_some("confirmed");
    sqlx::query(
        "INSERT INTO media_uploads \
             (id, user_id, object_key, scope, target_id, content_type, byte_size, status, \
              confirmed_at, expires_at, created_at) \
         VALUES ($1, $2, $3, 'chat', $4, 'image/png', 64, $5, \
                 CASE WHEN $6::TEXT IS NULL THEN NULL ELSE clock_timestamp() END, \
                 clock_timestamp() + INTERVAL '1 day', \
                 clock_timestamp() - INTERVAL '1 second')",
    )
    .bind(upload_id)
    .bind(user_id)
    .bind(object_key)
    .bind(target_id)
    .bind(status)
    .bind(confirmed_at)
    .execute(pool)
    .await?;
    Ok(())
}

async fn database_snapshot(pool: &PgPool) -> TestResult<DatabaseSnapshot> {
    let snapshot = sqlx::query_scalar::<_, String>(
        "SELECT jsonb_build_object( \
            'users', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM users r), '[]'), \
            'groups', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM groups r), '[]'), \
            'memberships', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM memberships r), '[]'), \
            'chatrooms', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM chatrooms r), '[]'), \
            'messages', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM messages r), '[]'), \
            'conversation_events', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM conversation_events r), '[]'), \
            'outbox_events', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM outbox_events r), '[]'), \
            'auth_identities', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM auth_identities r), '[]'), \
            'refresh_sessions', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM refresh_sessions r), '[]'), \
            'invites', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM invites r), '[]'), \
            'chatroom_reads', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM chatroom_reads r), '[]'), \
            'topics', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM topics r), '[]'), \
            'topic_media', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM topic_media r), '[]'), \
            'media_uploads', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM media_uploads r), '[]'), \
            'message_media', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM message_media r), '[]'), \
            'notifications', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM notifications r), '[]'), \
            'push_installations', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM push_installations r), '[]'), \
            'push_delivery_intents', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM push_delivery_intents r), '[]'), \
            'anonymous_author_tombstones', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.user_id) FROM anonymous_author_tombstones r), '[]'), \
            'account_object_deletion_intents', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM account_object_deletion_intents r), '[]') \
        )::TEXT",
    )
    .fetch_one(pool)
    .await?;
    Ok(DatabaseSnapshot(snapshot))
}

async fn tombstone_projection(pool: &PgPool) -> TestResult<(Uuid, String, Option<String>)> {
    let mut rows = sqlx::query_as::<_, (Uuid, String, Option<String>)>(
        "SELECT tombstone.user_id, users.nickname, users.avatar_url \
         FROM anonymous_author_tombstones tombstone \
         INNER JOIN users ON users.id = tombstone.user_id \
         ORDER BY tombstone.user_id",
    )
    .fetch_all(pool)
    .await?;
    if rows.len() != 1 {
        return Err(test_error(format!(
            "expected exactly one anonymous tombstone, found {}",
            rows.len()
        )));
    }
    rows.pop()
        .ok_or_else(|| test_error("anonymous tombstone projection disappeared"))
}

async fn require_no_private_references(pool: &PgPool, target_id: Uuid) -> TestResult {
    let references = sqlx::query_scalar::<_, i64>(
        "SELECT \
            (SELECT count(*) FROM users WHERE id = $1) + \
            (SELECT count(*) FROM auth_identities WHERE user_id = $1) + \
            (SELECT count(*) FROM refresh_sessions WHERE user_id = $1) + \
            (SELECT count(*) FROM memberships WHERE user_id = $1) + \
            (SELECT count(*) FROM invites WHERE created_by = $1) + \
            (SELECT count(*) FROM chatroom_reads WHERE user_id = $1) + \
            (SELECT count(*) FROM notifications WHERE user_id = $1) + \
            (SELECT count(*) FROM push_installations WHERE user_id = $1) + \
            (SELECT count(*) FROM push_delivery_intents WHERE recipient_user_id = $1) + \
            (SELECT count(*) FROM media_uploads WHERE user_id = $1) + \
            (SELECT count(*) FROM messages WHERE sender_id = $1) + \
            (SELECT count(*) FROM topics WHERE author_id = $1) + \
            (SELECT count(*) FROM groups WHERE owner_id = $1) + \
            (SELECT count(*) FROM anonymous_author_tombstones WHERE user_id = $1)",
    )
    .bind(target_id)
    .fetch_one(pool)
    .await?;
    require_eq(
        references,
        0,
        "the committed transition retained a direct private account reference",
    )
}

async fn require_retained_content(
    pool: &PgPool,
    fixture: &FullDeletionFixture,
    tombstone_id: Uuid,
) -> TestResult {
    let message = sqlx::query_as::<_, (Option<Uuid>, Option<String>)>(
        "SELECT sender_id, body FROM messages WHERE id = $1",
    )
    .bind(fixture.message_id)
    .fetch_one(pool)
    .await?;
    require_eq(
        message,
        (Some(tombstone_id), Some("retained message body".to_owned())),
        "retained message identity or body changed",
    )?;
    let topic = sqlx::query_as::<_, (Uuid, String, Option<String>)>(
        "SELECT author_id, title, body FROM topics WHERE id = $1",
    )
    .bind(fixture.topic_id)
    .fetch_one(pool)
    .await?;
    require_eq(
        topic,
        (
            tombstone_id,
            "retained topic".to_owned(),
            Some("retained topic body".to_owned()),
        ),
        "retained topic identity or content changed",
    )?;
    let upload = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT user_id, object_key, status FROM media_uploads WHERE id = $1",
    )
    .bind(fixture.bound_upload_id)
    .fetch_one(pool)
    .await?;
    require_eq(
        upload,
        (
            tombstone_id,
            fixture.bound_object_key.clone(),
            "bound".to_owned(),
        ),
        "bound retained upload was deleted or changed",
    )?;
    require_eq(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM message_media \
             WHERE message_id = $1 AND media_upload_id = $2 AND object_key = $3",
        )
        .bind(fixture.message_id)
        .bind(fixture.bound_upload_id)
        .bind(&fixture.bound_object_key)
        .fetch_one(pool)
        .await?,
        1,
        "retained message-media binding changed",
    )?;
    require_eq(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM conversation_events \
             WHERE id = $1 \
               AND payload ->> 'sender_id' = $2 \
               AND payload ->> 'sender_nickname' = $3",
        )
        .bind(fixture.event_id)
        .bind(tombstone_id.to_string())
        .bind(ANONYMOUS_AUTHOR_NICKNAME)
        .fetch_one(pool)
        .await?,
        1,
        "retained conversation event was deleted or not anonymized",
    )?;
    require_eq(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM outbox_events \
             WHERE id = $1 \
               AND conversation_event_id = $2 \
               AND payload ->> 'sender_id' = $3 \
               AND payload ->> 'sender_nickname' = $4",
        )
        .bind(fixture.conversation_outbox_id)
        .bind(fixture.event_id)
        .bind(tombstone_id.to_string())
        .bind(ANONYMOUS_AUTHOR_NICKNAME)
        .fetch_one(pool)
        .await?,
        1,
        "retained conversation outbox row was deleted or not anonymized",
    )
}

async fn require_cleanup_intents(pool: &PgPool) -> TestResult {
    let actual = sqlx::query_as::<_, (String, String)>(
        "SELECT object_key, status FROM account_object_deletion_intents ORDER BY object_key",
    )
    .fetch_all(pool)
    .await?;
    let expected = UNBOUND_OBJECT_KEYS
        .iter()
        .map(|object_key| ((*object_key).to_owned(), "pending".to_owned()))
        .collect::<Vec<_>>();
    require_eq(
        actual,
        expected,
        "unbound objects did not produce one unique pending cleanup intent each",
    )?;
    require_eq(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM media_uploads WHERE object_key IN ($1, $2, $3)",
        )
        .bind(UNBOUND_OBJECT_KEYS[0])
        .bind(UNBOUND_OBJECT_KEYS[1])
        .bind(UNBOUND_OBJECT_KEYS[2])
        .fetch_one(pool)
        .await?,
        0,
        "an unbound media-upload row survived cleanup-intent creation",
    )
}

async fn require_payloads_scrubbed(pool: &PgPool, target_id: Uuid) -> TestResult {
    let private_avatar = format!("https://private.invalid/{target_id}");
    let leaked = sqlx::query_scalar::<_, i64>(
        "SELECT \
            (SELECT count(*) FROM conversation_events \
             WHERE payload::TEXT LIKE '%' || $1::TEXT || '%' \
                OR payload::TEXT LIKE '%' || $2 || '%' \
                OR payload::TEXT LIKE '%' || $3 || '%') + \
            (SELECT count(*) FROM outbox_events \
             WHERE intent_type = 'conversation' \
               AND (payload::TEXT LIKE '%' || $1::TEXT || '%' \
                    OR payload::TEXT LIKE '%' || $2 || '%' \
                    OR payload::TEXT LIKE '%' || $3 || '%'))",
    )
    .bind(target_id)
    .bind("private target nickname")
    .bind(private_avatar)
    .fetch_one(pool)
    .await?;
    require_eq(
        leaked,
        0,
        "retained event or outbox payload leaked the deleted account id",
    )
}

async fn control_protocol_snapshot(pool: &PgPool, outbox_id: Uuid) -> TestResult<Value> {
    let row = sqlx::query(
        "SELECT id, intent_type, event_type, event_version, aggregate_type, aggregate_id, \
                conversation_event_id, payload::TEXT AS payload_text \
         FROM outbox_events WHERE id = $1",
    )
    .bind(outbox_id)
    .fetch_one(pool)
    .await?;
    Ok(json!({
        "id": row.try_get::<Uuid, _>("id")?,
        "intent_type": row.try_get::<String, _>("intent_type")?,
        "event_type": row.try_get::<String, _>("event_type")?,
        "event_version": row.try_get::<i16, _>("event_version")?,
        "aggregate_type": row.try_get::<String, _>("aggregate_type")?,
        "aggregate_id": row.try_get::<Uuid, _>("aggregate_id")?,
        "conversation_event_id": row.try_get::<Option<Uuid>, _>("conversation_event_id")?,
        "payload_text": row.try_get::<String, _>("payload_text")?,
    }))
}

async fn unrelated_control_snapshot(
    pool: &PgPool,
    outbox_id: Uuid,
) -> TestResult<(String, String, String, i64)> {
    Ok(sqlx::query_as::<_, (String, String, String, i64)>(
        "SELECT event_type, payload::TEXT, status, claim_generation \
         FROM outbox_events WHERE id = $1",
    )
    .bind(outbox_id)
    .fetch_one(pool)
    .await?)
}

async fn replace_membership_id(
    pool: &PgPool,
    group_id: Uuid,
    user_id: Uuid,
    membership_id: Uuid,
) -> TestResult {
    let affected =
        sqlx::query("UPDATE memberships SET id = $3 WHERE group_id = $1 AND user_id = $2")
            .bind(group_id)
            .bind(user_id)
            .bind(membership_id)
            .execute(pool)
            .await?
            .rows_affected();
    require_eq(
        affected,
        1,
        "ordered fixture did not replace exactly one target membership id",
    )
}

fn mutex_guard<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

async fn finish_database_test(
    database: TestDatabase,
    pool: PgPool,
    test_result: TestResult,
) -> TestResult {
    pool.close().await;
    let cleanup_result = database.dispose().await;
    match (test_result, cleanup_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(test_failure), Ok(())) => Err(test_failure),
        (Ok(()), Err(cleanup_failure)) => Err(cleanup_failure),
        (Err(test_failure), Err(cleanup_failure)) => Err(test_error(format!(
            "transition test failed: {test_failure}; database cleanup also failed: {cleanup_failure}"
        ))),
    }
}
