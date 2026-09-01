async fn count(pool: &PgPool, relation: &str) -> TestResult<i64> {
    match relation {
        "messages" => Ok(sqlx::query_scalar("SELECT count(*) FROM messages")
            .fetch_one(pool)
            .await?),
        "conversation_events" => Ok(
            sqlx::query_scalar("SELECT count(*) FROM conversation_events")
                .fetch_one(pool)
                .await?,
        ),
        "outbox_events" => Ok(sqlx::query_scalar("SELECT count(*) FROM outbox_events")
            .fetch_one(pool)
            .await?),
        "message_media" => Ok(sqlx::query_scalar("SELECT count(*) FROM message_media")
            .fetch_one(pool)
            .await?),
        "topics" => Ok(sqlx::query_scalar("SELECT count(*) FROM topics")
            .fetch_one(pool)
            .await?),
        "chatrooms" => Ok(sqlx::query_scalar("SELECT count(*) FROM chatrooms")
            .fetch_one(pool)
            .await?),
        "chatroom_reads" => Ok(sqlx::query_scalar("SELECT count(*) FROM chatroom_reads")
            .fetch_one(pool)
            .await?),
        "notifications" => Ok(sqlx::query_scalar("SELECT count(*) FROM notifications")
            .fetch_one(pool)
            .await?),
        "notifications WHERE read_at IS NULL" => Ok(sqlx::query_scalar(
            "SELECT count(*) FROM notifications WHERE read_at IS NULL",
        )
        .fetch_one(pool)
        .await?),
        "push_delivery_intents" => Ok(sqlx::query_scalar(
            "SELECT count(*) FROM push_delivery_intents",
        )
        .fetch_one(pool)
        .await?),
        _ => Err(io::Error::other("unexpected durable relation").into()),
    }
}
async fn audio_upload_state(
    pool: &PgPool,
    upload_id: Uuid,
    status: &str,
    consumed: bool,
) -> TestResult<i64> {
    let query: &'static str = if consumed {
        "SELECT count(*) FROM media_uploads \
         WHERE id = $1 AND status = $2 \
           AND bound_message_id IS NOT NULL AND consumed_at IS NOT NULL"
    } else {
        "SELECT count(*) FROM media_uploads \
         WHERE id = $1 AND status = $2 \
           AND bound_message_id IS NULL AND consumed_at IS NULL"
    };
    Ok(sqlx::query_scalar(query)
        .bind(upload_id)
        .bind(status)
        .fetch_one(pool)
        .await?)
}
fn require(condition: bool, message: &str) -> TestResult {
    condition
        .then_some(())
        .ok_or_else(|| io::Error::other(message).into())
}
fn require_eq<T>(actual: T, expected: T, message: &str) -> TestResult
where
    T: std::fmt::Debug + PartialEq,
{
    if actual == expected {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{message}: actual={actual:?}, expected={expected:?}"
        ))
        .into())
    }
}
