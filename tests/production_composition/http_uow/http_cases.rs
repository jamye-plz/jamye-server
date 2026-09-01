const SECRET: &str = "task-12-production-auth-secret-at-least-32-bytes";
const ISSUER: &str = "jamye-task-12-test";
const AUDIENCE: &str = "jamye-task-12-client";

#[derive(Clone, Copy)]
enum Boundary {
    SendCore,
    SendMedia,
    SendNotification,
    TopicCore,
    TopicNotification,
    ReadMarker,
    ReadClear,
}

#[tokio::test]
async fn http_send_message_fault_after_message_event_outbox_rolls_back_then_retries() -> TestResult
{
    run(Boundary::SendCore).await
}
#[tokio::test]
async fn http_send_message_fault_after_media_binding_rolls_back_then_retries() -> TestResult {
    run(Boundary::SendMedia).await
}
#[tokio::test]
async fn http_send_message_fault_after_notification_push_rolls_back_then_retries() -> TestResult {
    run(Boundary::SendNotification).await
}
#[tokio::test]
async fn http_create_topic_fault_after_topic_chatroom_bootstrap_announcement_read_rolls_back_then_retries()
-> TestResult {
    run(Boundary::TopicCore).await
}
#[tokio::test]
async fn http_create_topic_fault_after_notification_push_rolls_back_then_retries() -> TestResult {
    run(Boundary::TopicNotification).await
}
#[tokio::test]
async fn http_mark_conversation_read_fault_after_marker_rolls_back_then_retries() -> TestResult {
    run(Boundary::ReadMarker).await
}
#[tokio::test]
async fn http_mark_conversation_read_fault_after_notification_clear_rolls_back_then_retries()
-> TestResult {
    run(Boundary::ReadClear).await
}

#[tokio::test]
async fn http_bodyless_zero_uploads_preserves_the_existing_content_error() -> TestResult {
    assert_bodyless_error(Vec::new(), "message_content_required").await
}

#[tokio::test]
async fn http_bodyless_multiple_uploads_preserves_the_existing_media_error() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let fixture = PostgresFixture::new(pool.clone()).await?;
        let extra_upload_id = insert_upload(
            &pool,
            fixture.send.message.sender_id,
            fixture.topic_chatroom_id,
            true,
        )
        .await?;
        let before = DurableRows::load(&pool, fixture.topic_chatroom_id).await?;
        let response = send_message(
            production_router(&app_config_for(&pool).await?, &auth_config()?)?,
            fixture.send.message.sender_id,
            fixture.topic_chatroom_id,
            Uuid::new_v4(),
            None,
            vec![fixture.audio_upload_id, extra_upload_id],
        )
        .await?;
        require_error(
            &response,
            StatusCode::UNPROCESSABLE_ENTITY,
            "media_not_available",
        )?;
        require(
            DurableRows::load(&pool, fixture.topic_chatroom_id).await? == before,
            "bodyless multiple-upload rejection mutated durable state",
        )
    }
    .await;
    pool.close().await;
    database.dispose().await?;
    result
}
#[tokio::test]
async fn http_bodyless_nonfinalized_upload_preserves_the_existing_media_error() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let fixture = PostgresFixture::new(pool.clone()).await?;
        let pending_upload_id = insert_upload(
            &pool,
            fixture.send.message.sender_id,
            fixture.topic_chatroom_id,
            false,
        )
        .await?;
        let before = DurableRows::load(&pool, fixture.topic_chatroom_id).await?;
        let response = send_message(
            production_router(&app_config_for(&pool).await?, &auth_config()?)?,
            fixture.send.message.sender_id,
            fixture.topic_chatroom_id,
            Uuid::new_v4(),
            None,
            vec![pending_upload_id],
        )
        .await?;
        require_error(
            &response,
            StatusCode::UNPROCESSABLE_ENTITY,
            "media_not_available",
        )?;
        require(
            DurableRows::load(&pool, fixture.topic_chatroom_id).await? == before,
            "bodyless nonfinalized-upload rejection mutated durable state",
        )
    }
    .await;
    pool.close().await;
    database.dispose().await?;
    result
}

#[tokio::test]
async fn http_bodyless_unauthorized_upload_preserves_the_existing_media_error() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let fixture = PostgresFixture::new(pool.clone()).await?;
        let before = DurableRows::load(&pool, fixture.topic_chatroom_id).await?;
        let response = send_message(
            production_router(&app_config_for(&pool).await?, &auth_config()?)?,
            fixture.recipient_id,
            fixture.topic_chatroom_id,
            Uuid::new_v4(),
            None,
            vec![fixture.audio_upload_id],
        )
        .await?;
        require_error(
            &response,
            StatusCode::UNPROCESSABLE_ENTITY,
            "media_not_available",
        )?;
        require(
            DurableRows::load(&pool, fixture.topic_chatroom_id).await? == before,
            "bodyless unauthorized-upload rejection mutated durable state",
        )
    }
    .await;
    pool.close().await;
    database.dispose().await?;
    result
}

#[tokio::test]
async fn http_topic_message_reuses_the_exact_canonical_event_for_one_notification_and_push()
-> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let fixture = PostgresFixture::new(pool.clone()).await?;
        let client_msg_id = Uuid::new_v4();
        let router = production_router(&app_config_for(&pool).await?, &auth_config()?)?;

        let created = send_message(
            router.clone(),
            fixture.send.message.sender_id,
            fixture.topic_chatroom_id,
            client_msg_id,
            None,
            vec![fixture.audio_upload_id],
        )
        .await?;
        require(created.status == StatusCode::CREATED, "topic message did not return Created")?;
        let retried = send_message(
            router,
            fixture.send.message.sender_id,
            fixture.topic_chatroom_id,
            client_msg_id,
            None,
            vec![fixture.audio_upload_id],
        )
        .await?;
        require(
            retried.status == StatusCode::OK,
            &format!(
                "topic message retry did not return Existing: status={} body={}",
                retried.status, retried.body
            ),
        )?;

        let message_id = response_id(&created.body, "message")?;
        let (source_event_id, source_cursor): (Uuid, i64) = sqlx::query_as(
            "SELECT id, cursor FROM conversation_events \
             WHERE conversation_id = $1 AND event_type = 'message.created' \
               AND payload ->> 'id' = $2::uuid::text",
        )
        .bind(fixture.topic_chatroom_id)
        .bind(message_id)
        .fetch_one(&pool)
        .await?;
        let (notifications, pushes, exact_sources): (i64, i64, i64) = sqlx::query_as(
            "SELECT \
                 (SELECT count(*) FROM notifications \
                  WHERE conversation_id = $1 AND source_cursor = $2 \
                    AND topic_id IS NOT NULL AND type = 'chat_unread'), \
                 (SELECT count(*) FROM push_delivery_intents WHERE source_message_id = $3), \
                 (SELECT count(*) FROM push_delivery_intents \
                  WHERE source_message_id = $3 AND source_event_id = $4)",
        )
        .bind(fixture.topic_chatroom_id)
        .bind(source_cursor)
        .bind(message_id)
        .bind(source_event_id)
        .fetch_one(&pool)
        .await?;
        require(
            notifications == 1 && pushes == 1 && exact_sources == 1,
            "topic message did not retain exactly one notification/push using its canonical message.created event",
        )
    }
    .await;
    pool.close().await;
    database.dispose().await?;
    result
}

#[tokio::test]
async fn http_create_topic_push_uses_topic_created_not_announcement_message_created() -> TestResult
{
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let fixture = PostgresFixture::new(pool.clone()).await?;
        let router = production_router(&app_config_for(&pool).await?, &auth_config()?)?;
        let path = format!("/api/v1/groups/{}/topics", fixture.topic.topic.group_id);
        let created = post_json(
            router.clone(),
            fixture.topic.topic.author_id,
            &path,
            Some(fixture.topic.topic.idempotency_key),
            json!({"title": fixture.topic.topic.title.clone()}),
        )
        .await?;
        require(
            created.status == StatusCode::CREATED,
            &format!(
                "topic did not return Created: status={} body={}",
                created.status, created.body
            ),
        )?;
        let retried = post_json(
            router,
            fixture.topic.topic.author_id,
            &path,
            Some(fixture.topic.topic.idempotency_key),
            json!({"title": fixture.topic.topic.title.clone()}),
        )
        .await?;
        require(
            retried.status == StatusCode::OK,
            &format!(
                "topic retry did not return Existing: status={} body={}",
                retried.status, retried.body
            ),
        )?;

        let topic_id = response_id(&created.body, "topic")?;
        let topic_chatroom_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM chatrooms WHERE type = 'topic' AND topic_id = $1",
        )
        .bind(topic_id)
        .fetch_one(&pool)
        .await?;
        let topic_event_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM conversation_events \
             WHERE conversation_id = $1 AND event_type = 'topic.created' \
               AND payload ->> 'topic_id' = $2::uuid::text",
        )
        .bind(topic_chatroom_id)
        .bind(topic_id)
        .fetch_one(&pool)
        .await?;
        let announcement_event_id: Uuid = sqlx::query_scalar(
            "SELECT event.id FROM conversation_events event \
             JOIN messages message ON message.id = (event.payload ->> 'id')::uuid \
             WHERE event.conversation_id = $1 AND event.event_type = 'message.created' \
               AND message.sender_id = $2 ORDER BY event.cursor DESC LIMIT 1",
        )
        .bind(fixture.main_chatroom_id)
        .bind(fixture.topic.topic.author_id)
        .fetch_one(&pool)
        .await?;
        let exact_pushes: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM push_delivery_intents \
             WHERE source_event_id = $1 AND recipient_user_id = $2 AND push_installation_id = $3",
        )
        .bind(topic_event_id)
        .bind(fixture.recipient_id)
        .bind(fixture.push_installation_id)
        .fetch_one(&pool)
        .await?;
        require(
            topic_event_id != announcement_event_id && exact_pushes == 1,
            "CreateTopic push used the announcement message.created event instead of the authoritative topic.created event",
        )
    }
    .await;
    pool.close().await;
    database.dispose().await?;
    result
}

#[tokio::test]
async fn http_main_chat_message_succeeds_without_a_topic_notification_or_push() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let fixture = PostgresFixture::new(pool.clone()).await?;
        let client_msg_id = Uuid::new_v4();
        let router = production_router(&app_config_for(&pool).await?, &auth_config()?)?;
        let created = send_message(
            router.clone(),
            fixture.send.message.sender_id,
            fixture.main_chatroom_id,
            client_msg_id,
            Some("main-chat message"),
            Vec::new(),
        )
        .await?;
        require(
            created.status == StatusCode::CREATED,
            "main-chat message did not return Created",
        )?;
        let retried = send_message(
            router,
            fixture.send.message.sender_id,
            fixture.main_chatroom_id,
            client_msg_id,
            Some("main-chat message"),
            Vec::new(),
        )
        .await?;
        require(
            retried.status == StatusCode::OK,
            "main-chat retry did not return Existing",
        )?;
        let message_id = response_id(&created.body, "message")?;
        let (notifications, pushes): (i64, i64) = sqlx::query_as(
            "SELECT \
                 (SELECT count(*) FROM notifications WHERE conversation_id = $1), \
                 (SELECT count(*) FROM push_delivery_intents WHERE source_message_id = $2)",
        )
        .bind(fixture.main_chatroom_id)
        .bind(message_id)
        .fetch_one(&pool)
        .await?;
        require(
            notifications == 0 && pushes == 0,
            "main-chat message fabricated a topic notification or push occurrence",
        )
    }
    .await;
    pool.close().await;
    database.dispose().await?;
    result
}

#[tokio::test]
async fn http_inconsistent_topic_chat_topology_fails_closed_before_notification_persistence()
-> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let fixture = PostgresFixture::new(pool.clone()).await?;
        let mismatched_group_id = Uuid::new_v4();
        sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, $2, $3)")
            .bind(mismatched_group_id)
            .bind("Task-12 inconsistent topology")
            .bind(fixture.send.message.sender_id)
            .execute(&pool)
            .await?;
        sqlx::query(
            "UPDATE topics SET group_id = $1 \
             WHERE id = (SELECT topic_id FROM chatrooms WHERE id = $2)",
        )
        .bind(mismatched_group_id)
        .bind(fixture.topic_chatroom_id)
        .execute(&pool)
        .await?;
        let before = DurableRows::load(&pool, fixture.topic_chatroom_id).await?;
        let response = send_message(
            production_router(&app_config_for(&pool).await?, &auth_config()?)?,
            fixture.send.message.sender_id,
            fixture.topic_chatroom_id,
            Uuid::new_v4(),
            Some("corrupt topology"),
            Vec::new(),
        )
        .await?;
        require_error(
            &response,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        )?;
        require(
            DurableRows::load(&pool, fixture.topic_chatroom_id).await? == before,
            "inconsistent topology created a message, notification, or push instead of failing closed",
        )
    }
    .await;
    pool.close().await;
    database.dispose().await?;
    result
}
