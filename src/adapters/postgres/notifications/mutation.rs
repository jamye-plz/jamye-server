use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::ports::push::{
    MarkNotificationReadCommand, NotificationReadRecord, NotificationsRepositoryError,
};

use super::database_error;

pub(super) async fn mark_notification_read(
    connection: &mut PgConnection,
    command: &MarkNotificationReadCommand,
) -> Result<NotificationReadRecord, NotificationsRepositoryError> {
    let row = sqlx::query_as::<_, (Uuid, OffsetDateTime)>(
        "UPDATE notifications \
         SET read_at = COALESCE(read_at, clock_timestamp()) \
         WHERE id = $1 AND user_id = $2 \
         RETURNING id, read_at",
    )
    .bind(command.notification_id)
    .bind(command.user_id)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("notification_mark_read", error))?
    .ok_or(NotificationsRepositoryError::NotificationNotFound)?;
    Ok(NotificationReadRecord {
        notification_id: row.0,
        read_at: row.1,
    })
}
