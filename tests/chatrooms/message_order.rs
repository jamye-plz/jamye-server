use std::{io, time::Duration};

use jamye_server::application::{
    chatrooms::{HistoryPageInput, ReadAnchorInput},
    messaging::SendMessageInput,
    topics::TopicCreateInput,
};
use sqlx::Connection;
use time::OffsetDateTime;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use crate::{
    TestResult,
    chatroom_helpers::{harness, insert_message_created_event, insert_user_message, topology},
    postgres_support::TestDatabase,
};

fn send_input(chatroom_id: Uuid, body: &str) -> SendMessageInput {
    SendMessageInput {
        chatroom_id,
        client_msg_id: Uuid::new_v4(),
        body: Some(body.to_owned()),
        media_upload_ids: Vec::new(),
        idempotency_key: None,
    }
}

#[tokio::test]
async fn concurrent_sends_keep_c2_history_and_c3_anchors_in_the_same_order() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let harness = harness(pool.clone());
    // Pause after the first timestamp is allocated but before its event cursor.
    // This deliberately exercises the interleaving raised in PR #2.
    sqlx::raw_sql(
        "CREATE FUNCTION hold_first_message() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN
           IF NEW.body = 'blocked-first' THEN
             PERFORM pg_advisory_xact_lock(132002);
           END IF;
           RETURN NEW;
         END $$;
         CREATE TRIGGER hold_first_message AFTER INSERT ON messages
         FOR EACH ROW EXECUTE FUNCTION hold_first_message();",
    )
    .execute(&pool)
    .await?;
    let mut gate = database.connection().await?;
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut gate)
        .await?;
    sqlx::query("SELECT pg_advisory_lock(132002)")
        .execute(&mut gate)
        .await?;
    let first_service = harness.compositions.clone();
    let first = tokio::spawn(async move {
        first_service
            .send_message_http(
                fixture.owner_id,
                send_input(fixture.chatroom_id, "blocked-first"),
            )
            .await
    });
    let reached_gate = timeout(Duration::from_secs(10), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity
                 WHERE datname = current_database() AND $1 = ANY(pg_blocking_pids(pid)))",
            )
            .bind(gate_pid)
            .fetch_one(&pool)
            .await?;
            if waiting {
                return Ok::<_, sqlx::Error>(());
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    let second_service = harness.compositions.clone();
    let second = tokio::spawn(async move {
        second_service
            .send_message_http(fixture.owner_id, send_input(fixture.chatroom_id, "second"))
            .await
    });
    // Wait until the second request either overtakes the first (old behavior)
    // or is demonstrably queued behind it. No scheduling-dependent sleep/assert.
    let second_observed = timeout(Duration::from_secs(10), async {
        loop {
            let queued: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity
                 WHERE datname = current_database() AND wait_event = 'advisory'
                   AND cardinality(pg_blocking_pids(pid)) > 0
                   AND NOT ($1 = ANY(pg_blocking_pids(pid))))",
            )
            .bind(gate_pid)
            .fetch_one(&pool)
            .await?;
            if second.is_finished() || queued {
                return Ok::<_, sqlx::Error>(());
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    // A blocked room must not prevent an independent room from sending.
    let independent = timeout(
        Duration::from_secs(10),
        harness.compositions.send_message_http(
            fixture.outsider_id,
            send_input(fixture.other_chatroom_id, "independent-room"),
        ),
    )
    .await;
    sqlx::query("SELECT pg_advisory_unlock(132002)")
        .execute(&mut gate)
        .await?;
    gate.close().await?;
    let first_result = timeout(Duration::from_secs(10), first).await;
    let second_result = timeout(Duration::from_secs(10), second).await;
    let checks: TestResult = async {
        reached_gate??;
        second_observed??;
        independent??;
        first_result???;
        second_result???;
        let history = harness
            .service
            .message_history(
                fixture.owner_id,
                fixture.chatroom_id,
                HistoryPageInput {
                    before: None,
                    limit: Some(10),
                },
            )
            .await?;
        let event_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT (payload ->> 'id')::uuid FROM conversation_events
             WHERE conversation_id = $1 AND event_type = 'message.created'
               AND event_version = 1 ORDER BY cursor",
        )
        .bind(fixture.chatroom_id)
        .fetch_all(&pool)
        .await?;
        let history_ids: Vec<_> = history.items.iter().map(|item| item.message.id).collect();
        if history_ids != event_ids || history_ids.len() != 2 {
            return Err(io::Error::other(
                "C2 history order differs from message.created cursor order",
            )
            .into());
        }
        let marker = harness
            .service
            .mark_read_anchor(
                fixture.owner_id,
                fixture.chatroom_id,
                ReadAnchorInput::MessageId(history_ids[1]),
            )
            .await?;
        let latest_cursor: i64 = sqlx::query_scalar(
            "SELECT max(cursor) FROM conversation_events WHERE conversation_id = $1",
        )
        .bind(fixture.chatroom_id)
        .fetch_one(&pool)
        .await?;
        if marker.last_read_cursor != latest_cursor {
            return Err(io::Error::other(
                "reading the complete C2 page left an earlier message unread",
            )
            .into());
        }
        Ok(())
    }
    .await;
    pool.close().await;
    database.dispose().await?;
    checks
}

#[tokio::test]
async fn normal_and_topic_announcement_writers_advance_past_the_room_timestamp() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let harness = harness(pool.clone());
    // Simulate a backwards wall clock; a timestamp tie must also be impossible.
    let future = OffsetDateTime::now_utc() + time::Duration::days(1);
    let seed =
        insert_user_message(&pool, fixture.chatroom_id, fixture.owner_id, "seed", future).await?;
    insert_message_created_event(&pool, fixture.chatroom_id, seed).await?;
    harness
        .compositions
        .send_message_http(
            fixture.owner_id,
            send_input(fixture.chatroom_id, "after-seed"),
        )
        .await?;
    harness
        .compositions
        .create_topic_http(
            fixture.owner_id,
            fixture.group_id,
            TopicCreateInput {
                idempotency_key: Uuid::new_v4(),
                title: "Clock regression topic".to_owned(),
            },
        )
        .await?;
    let timestamps: Vec<OffsetDateTime> = sqlx::query_scalar(
        "SELECT m.created_at FROM messages m JOIN conversation_events e
         ON e.conversation_id = m.chatroom_id AND e.payload ->> 'id' = m.id::text
         WHERE m.chatroom_id = $1 AND e.event_type = 'message.created' AND e.event_version = 1
         ORDER BY e.cursor",
    )
    .bind(fixture.chatroom_id)
    .fetch_all(&pool)
    .await?;
    pool.close().await;
    database.dispose().await?;
    assert_eq!(timestamps.len(), 3);
    assert!(
        timestamps.windows(2).all(|pair| pair[0] < pair[1]),
        "both production message writers must use strictly increasing room timestamps"
    );
    Ok(())
}
