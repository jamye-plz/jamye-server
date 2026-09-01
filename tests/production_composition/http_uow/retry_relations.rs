async fn retry_relations(
    pool: &PgPool,
    fixture: &PostgresFixture,
    boundary: Boundary,
    body: &str,
) -> TestResult {
    match boundary {
        Boundary::SendCore | Boundary::SendMedia | Boundary::SendNotification => {
            let message_id: Uuid = sqlx::query_scalar(
                "SELECT id FROM messages WHERE chatroom_id = $1 AND client_msg_id = $2",
            )
            .bind(fixture.topic_chatroom_id)
            .bind(fixture.send.message.client_msg_id)
            .fetch_one(pool)
            .await?;
            require(
                body.contains(&message_id.to_string()),
                "send retry response did not identify the canonical message",
            )?;
            require(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM message_media WHERE message_id = $1 AND media_upload_id = $2").bind(message_id).bind(fixture.audio_upload_id).fetch_one(pool).await? == 1, "send retry did not retain exactly one media foreign-key binding")?;
            require(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM conversation_events event JOIN outbox_events outbox ON outbox.conversation_event_id = event.id WHERE event.conversation_id = $1 AND event.payload ->> 'id' = $2::uuid::text AND outbox.event_type = 'message.created'").bind(fixture.topic_chatroom_id).bind(message_id).fetch_one(pool).await? == 1, "send retry did not retain one canonical event/outbox relation")?;
            require(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM media_uploads WHERE id = $1 AND status = 'bound' AND bound_message_id = $2 AND consumed_at IS NOT NULL").bind(fixture.audio_upload_id).bind(message_id).fetch_one(pool).await? == 1, "send retry did not retain the consumed upload state")?;
            require(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM push_delivery_intents WHERE source_message_id = $1 AND recipient_user_id = $2 AND push_installation_id = $3").bind(message_id).bind(fixture.recipient_id).bind(fixture.push_installation_id).fetch_one(pool).await? == 1, "send retry did not retain exactly one recipient push relation")
        }
        Boundary::TopicCore | Boundary::TopicNotification => {
            let command = &fixture.topic.topic;
            let topic_id = response_id(body, "topic")?;
            require(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM topics WHERE id = $1 AND group_id = $2 AND author_id = $3 AND idempotency_key = $4 AND title = $5").bind(topic_id).bind(command.group_id).bind(command.author_id).bind(command.idempotency_key).bind(&command.title).fetch_one(pool).await? == 1, "topic retry did not retain the requested topic identity")?;
            let topic_chatroom_id: Uuid = sqlx::query_scalar(
                "SELECT id FROM chatrooms WHERE group_id = $1 AND type = 'topic' AND topic_id = $2",
            )
            .bind(command.group_id)
            .bind(topic_id)
            .fetch_one(pool)
            .await?;
            require(
                sqlx::query_scalar::<_, i64>(
                    "SELECT count(*) FROM chatroom_reads WHERE user_id = $1 AND chatroom_id = $2",
                )
                .bind(command.author_id)
                .bind(topic_chatroom_id)
                .fetch_one(pool)
                .await?
                    == 1,
                "topic retry did not retain the author read marker",
            )?;
            let topic_event_id: Uuid = sqlx::query_scalar("SELECT id FROM conversation_events WHERE conversation_id = $1 AND event_type = 'topic.created' AND payload ->> 'topic_id' = $2::uuid::text").bind(topic_chatroom_id).bind(topic_id).fetch_one(pool).await?;
            let announcement_message_id: Uuid = sqlx::query_scalar("SELECT id FROM messages WHERE chatroom_id = $1 AND sender_id = $2 ORDER BY created_at DESC LIMIT 1").bind(fixture.main_chatroom_id).bind(command.author_id).fetch_one(pool).await?;
            let announcement_event_id: Uuid = sqlx::query_scalar("SELECT id FROM conversation_events WHERE conversation_id = $1 AND event_type = 'message.created' AND payload ->> 'id' = $2::uuid::text").bind(fixture.main_chatroom_id).bind(announcement_message_id).fetch_one(pool).await?;
            require(
                sqlx::query_scalar::<_, i64>(
                    "SELECT count(*) FROM outbox_events WHERE conversation_event_id = ANY($1)",
                )
                .bind(vec![topic_event_id, announcement_event_id])
                .fetch_one(pool)
                .await?
                    == 2,
                "topic retry did not retain both event/outbox foreign-key relations",
            )?;
            require(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM push_delivery_intents WHERE source_event_id = $1 AND recipient_user_id = $2 AND push_installation_id = $3").bind(topic_event_id).bind(fixture.recipient_id).bind(fixture.push_installation_id).fetch_one(pool).await? == 1, "topic retry did not retain exactly one recipient push relation")
        }
        Boundary::ReadMarker | Boundary::ReadClear => {
            let command = fixture.read.read;
            let (response_chatroom_id, response_cursor) = response_read_marker(body)?;
            require(
                response_chatroom_id == command.chatroom_id && response_cursor == command.cursor,
                "read retry response did not preserve the requested chatroom/cursor",
            )?;
            let marker_ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM chatroom_reads WHERE user_id = $1 AND chatroom_id = $2 AND last_read_cursor = $3 ORDER BY id").bind(command.user_id).bind(command.chatroom_id).bind(command.cursor).fetch_all(pool).await?;
            let [marker_id] = marker_ids.as_slice() else {
                return Err(format!(
                    "read retry did not produce one exact durable marker: found {}",
                    marker_ids.len()
                )
                .into());
            };
            let marker_id = *marker_id;
            require(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM chatroom_reads WHERE id = $1 AND user_id = $2 AND chatroom_id = $3 AND last_read_cursor = $4").bind(marker_id).bind(command.user_id).bind(command.chatroom_id).bind(command.cursor).fetch_one(pool).await? == 1, "read retry did not retain the exact marker relation")?;
            require(
                sqlx::query_scalar::<_, i64>(
                    "SELECT count(*) FROM notifications WHERE id = $1 AND read_at IS NOT NULL",
                )
                .bind(fixture.seeded_notification_id)
                .fetch_one(pool)
                .await?
                    == 1,
                "read retry did not retain the bounded seeded-notification clear",
            )
        }
    }
}

fn response_id(body: &str, resource: &str) -> TestResult<Uuid> {
    let response: Value = serde_json::from_str(body)?;
    let id = response.get("id").and_then(Value::as_str).ok_or_else(|| {
        std::io::Error::other(format!("{resource} retry response did not contain an id"))
    })?;
    Ok(Uuid::try_parse(id)?)
}

fn response_read_marker(body: &str) -> TestResult<(Uuid, i64)> {
    let response: Value = serde_json::from_str(body)?;
    let chatroom_id = response
        .get("chatroom_id")
        .and_then(Value::as_str)
        .ok_or_else(|| std::io::Error::other("read retry response did not contain chatroom_id"))?;
    let cursor = response
        .get("last_read_cursor")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            std::io::Error::other("read retry response did not contain last_read_cursor")
        })?;
    Ok((Uuid::try_parse(chatroom_id)?, cursor.parse()?))
}

async fn app_config_for(pool: &PgPool) -> TestResult<AppConfig> {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(pool)
        .await?;
    let mut url = Url::parse(&env::var("DATABASE_URL")?)?;
    url.set_path(&format!("/{database}"));
    Ok(AppConfig::try_from(ConfigInput {
        environment: Some("test".to_owned()),
        readiness_timeout_ms: Some("1000".to_owned()),
        database_url: Some(url.to_string()),
        redis_url: Some("redis://127.0.0.1/".to_owned()),
        ..ConfigInput::default()
    })?)
}
