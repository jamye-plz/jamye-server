use std::{collections::HashSet, io};

use jamye_server::{
    application::chatrooms::{ChatroomPageInput, ChatroomsError, HistoryPageInput},
    domain::messaging::MessageKind,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    TestResult,
    chatroom_helpers::{
        harness, insert_chatroom, insert_system_message, insert_user_message, topology,
    },
    postgres_support::TestDatabase,
};

#[tokio::test]
async fn chatroom_pages_are_strictly_forward_complete_and_membership_safe() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let service = harness(pool.clone()).service;
    let tied_at = OffsetDateTime::UNIX_EPOCH + time::Duration::hours(1);
    for index in 0..5 {
        let topic_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO topics (id, group_id, author_id, idempotency_key, request_fingerprint, title) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(topic_id)
        .bind(fixture.group_id)
        .bind(fixture.owner_id)
        .bind(Uuid::new_v4())
        .bind("a".repeat(64))
        .bind(format!("페이지 주제 {index}"))
        .execute(&pool)
        .await?;
        insert_chatroom(&pool, fixture.group_id, "topic", Some(topic_id), tied_at).await?;
    }

    let expected = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM chatrooms WHERE group_id = $1 ORDER BY created_at, id",
    )
    .bind(fixture.group_id)
    .fetch_all(&pool)
    .await?;
    let mut actual = Vec::new();
    let mut after = None;
    loop {
        let page = service
            .list_chatrooms(
                fixture.owner_id,
                fixture.group_id,
                ChatroomPageInput {
                    after,
                    limit: Some(2),
                },
            )
            .await?;
        assert!(page.items.len() <= 2);
        assert!(
            page.items
                .iter()
                .all(|chatroom| chatroom.group_id == fixture.group_id)
        );
        actual.extend(page.items.into_iter().map(|chatroom| chatroom.id));
        let Some(cursor) = page.next_cursor else {
            break;
        };
        assert_eq!(actual.last().map(Uuid::to_string), Some(cursor.clone()));
        after = Some(cursor);
    }
    assert_eq!(actual, expected);
    assert_eq!(
        actual.iter().copied().collect::<HashSet<_>>().len(),
        actual.len()
    );

    let foreign_cursor = service
        .list_chatrooms(
            fixture.owner_id,
            fixture.group_id,
            ChatroomPageInput {
                after: Some(fixture.other_chatroom_id.to_string()),
                limit: Some(2),
            },
        )
        .await;
    assert_eq!(foreign_cursor, Err(ChatroomsError::RequestValidation));
    let outsider = service
        .list_chatrooms(
            fixture.outsider_id,
            fixture.group_id,
            ChatroomPageInput {
                after: Some(fixture.chatroom_id.to_string()),
                limit: Some(2),
            },
        )
        .await;
    assert_eq!(outsider, Err(ChatroomsError::MembershipRequired));

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn history_pages_are_chronological_denormalized_and_event_log_independent() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let service = harness(pool.clone()).service;
    let tied_at = OffsetDateTime::UNIX_EPOCH + time::Duration::hours(3);
    let mut inserted = Vec::new();
    for index in 0..5 {
        inserted.push(
            insert_user_message(
                &pool,
                fixture.chatroom_id,
                fixture.owner_id,
                &format!("히스토리 {index}"),
                tied_at,
            )
            .await?,
        );
    }
    let system_id =
        insert_system_message(&pool, fixture.chatroom_id, "시스템 메시지", tied_at).await?;
    inserted.push(system_id);
    let foreign_message = insert_user_message(
        &pool,
        fixture.other_chatroom_id,
        fixture.outsider_id,
        "다른 채팅방 비밀",
        tied_at,
    )
    .await?;

    let event_count: i64 = sqlx::query_scalar("SELECT count(*) FROM conversation_events")
        .fetch_one(&pool)
        .await?;
    assert_eq!(event_count, 0);
    let expected = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM messages WHERE chatroom_id = $1 ORDER BY created_at, id",
    )
    .bind(fixture.chatroom_id)
    .fetch_all(&pool)
    .await?;

    let mut pages = Vec::new();
    let mut seen = HashSet::new();
    let mut before = None;
    loop {
        let page = service
            .message_history(
                fixture.owner_id,
                fixture.chatroom_id,
                HistoryPageInput {
                    before,
                    limit: Some(2),
                },
            )
            .await?;
        let ids = page
            .items
            .iter()
            .map(|item| item.message.id)
            .collect::<Vec<_>>();
        for window in ids.windows(2) {
            let left = expected.iter().position(|id| id == &window[0]);
            let right = expected.iter().position(|id| id == &window[1]);
            assert!(left < right, "each history page must be chronological");
        }
        for item in &page.items {
            assert_eq!(item.message.chatroom_id, fixture.chatroom_id);
            assert!(item.message.media.is_empty());
            assert!(seen.insert(item.message.id), "history repeated a message");
            if item.message.id == system_id {
                assert_eq!(item.message.message_type, MessageKind::System);
                assert_eq!(item.message.sender_id, None);
                assert_eq!(item.sender_nickname, None);
                assert_eq!(item.sender_avatar_url, None);
            } else {
                assert_eq!(item.message.message_type, MessageKind::User);
                assert_eq!(item.sender_nickname.as_deref(), Some("채팅방 소유자"));
                assert_eq!(
                    item.sender_avatar_url.as_deref(),
                    Some("https://cdn.test/owner.png")
                );
            }
        }
        pages.extend(ids);
        let Some(cursor) = page.next_cursor else {
            break;
        };
        before = Some(cursor);
    }
    assert_eq!(seen.len(), inserted.len());
    assert_eq!(seen, expected.iter().copied().collect::<HashSet<_>>());
    assert_eq!(pages.len(), expected.len());

    let foreign_cursor = service
        .message_history(
            fixture.owner_id,
            fixture.chatroom_id,
            HistoryPageInput {
                before: Some(foreign_message.to_string()),
                limit: Some(2),
            },
        )
        .await;
    assert_eq!(foreign_cursor, Err(ChatroomsError::RequestValidation));
    let outsider = service
        .message_history(
            fixture.outsider_id,
            fixture.chatroom_id,
            HistoryPageInput {
                before: None,
                limit: Some(2),
            },
        )
        .await;
    assert_eq!(outsider, Err(ChatroomsError::MembershipRequired));

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn history_projects_poster_media_id_for_linked_videos_and_none_for_legacy_attachments()
-> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let service = harness(pool.clone()).service;
    let tied_at = OffsetDateTime::UNIX_EPOCH + time::Duration::hours(4);

    let with_poster_message = insert_user_message(
        &pool,
        fixture.chatroom_id,
        fixture.owner_id,
        "포스터 있음",
        tied_at,
    )
    .await?;
    let legacy_message = insert_user_message(
        &pool,
        fixture.chatroom_id,
        fixture.owner_id,
        "포스터 없음",
        tied_at + time::Duration::seconds(1),
    )
    .await?;

    let poster_id = Uuid::new_v4();
    let now = OffsetDateTime::now_utc();
    let confirmed_at = now - time::Duration::minutes(2);
    let consumed_at = now - time::Duration::minutes(1);
    let created_at = now - time::Duration::minutes(3);
    let expires_at = now + time::Duration::hours(1);
    sqlx::query(
        "INSERT INTO media_uploads \
             (id, user_id, object_key, scope, target_id, content_type, byte_size, \
              status, bound_message_id, confirmed_at, consumed_at, expires_at, created_at) \
         VALUES ($1, $2, $3, 'chat', $4, 'image/jpeg', 512, \
                 'bound', $5, $6, $7, $8, $9)",
    )
    .bind(poster_id)
    .bind(fixture.owner_id)
    .bind(format!("chat/{}/{poster_id}", fixture.chatroom_id))
    .bind(fixture.chatroom_id)
    .bind(with_poster_message)
    .bind(confirmed_at)
    .bind(consumed_at)
    .bind(expires_at)
    .bind(created_at)
    .execute(&pool)
    .await?;

    for (message_id, poster) in [
        (with_poster_message, Some(poster_id)),
        (legacy_message, None),
    ] {
        let video_id = Uuid::new_v4();
        let object_key = format!("chat/{}/{video_id}", fixture.chatroom_id);
        sqlx::query(
            "INSERT INTO media_uploads \
                 (id, user_id, object_key, scope, target_id, content_type, byte_size, \
                  status, bound_message_id, confirmed_at, consumed_at, expires_at, created_at, \
                  poster_upload_id) \
             VALUES ($1, $2, $3, 'chat', $4, 'video/mp4', 4096, \
                     'bound', $5, $6, $7, $8, $9, $10)",
        )
        .bind(video_id)
        .bind(fixture.owner_id)
        .bind(&object_key)
        .bind(fixture.chatroom_id)
        .bind(message_id)
        .bind(confirmed_at)
        .bind(consumed_at)
        .bind(expires_at)
        .bind(created_at)
        .bind(poster)
        .execute(&pool)
        .await?;
        sqlx::query(
            "INSERT INTO message_media \
                 (id, message_id, media_upload_id, type, object_key, byte_size, position, \
                  created_at) \
             VALUES ($1, $2, $3, 'video/mp4', $4, 4096, 0, $5)",
        )
        .bind(Uuid::new_v4())
        .bind(message_id)
        .bind(video_id)
        .bind(&object_key)
        .bind(consumed_at)
        .execute(&pool)
        .await?;
    }

    let page = service
        .message_history(
            fixture.owner_id,
            fixture.chatroom_id,
            HistoryPageInput {
                before: None,
                limit: Some(10),
            },
        )
        .await?;
    let with_poster_item = page
        .items
        .iter()
        .find(|item| item.message.id == with_poster_message)
        .ok_or_else(|| io::Error::other("with-poster message missing from history"))?;
    assert_eq!(with_poster_item.message.media.len(), 1);
    assert_eq!(
        with_poster_item.message.media[0].poster_media_id,
        Some(poster_id)
    );
    let legacy_item = page
        .items
        .iter()
        .find(|item| item.message.id == legacy_message)
        .ok_or_else(|| io::Error::other("legacy message missing from history"))?;
    assert_eq!(legacy_item.message.media.len(), 1);
    assert_eq!(legacy_item.message.media[0].poster_media_id, None);

    pool.close().await;
    database.dispose().await
}
