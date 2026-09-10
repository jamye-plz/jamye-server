use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

/// Allocate the next C2 history timestamp while serializing this room's message
/// and message.created writes through the caller's transaction completion.
/// All production message writers must call this after authorization/group locks.
pub(super) async fn next_timestamp(
    connection: &mut PgConnection,
    chatroom_id: Uuid,
) -> Result<OffsetDateTime, sqlx::Error> {
    sqlx::query(
        "SELECT pg_advisory_xact_lock(hashtextextended('jamye:message-order:' || $1::uuid::text, 0))",
    )
    .bind(chatroom_id)
    .execute(&mut *connection)
    .await?;
    // A separate READ COMMITTED statement sees the preceding writer's commit
    // after waiting for its lock. Strict advancement also handles clock rollback
    // and timestamp ties without changing the existing (created_at, id) C2 order.
    sqlx::query_scalar(
        "SELECT GREATEST(clock_timestamp(),
            (SELECT created_at + INTERVAL '1 microsecond' FROM messages
             WHERE chatroom_id = $1 ORDER BY created_at DESC, id DESC LIMIT 1))",
    )
    .bind(chatroom_id)
    .fetch_one(connection)
    .await
}
