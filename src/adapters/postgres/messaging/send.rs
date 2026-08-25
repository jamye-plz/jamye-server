use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    adapters::postgres::transactions::connection,
    domain::messaging::{
        CanonicalMessage, MessageCreatedEvent, MessageCreatedType, MessageKind, SendMessageCommand,
    },
    ports::{
        messaging::{MessagingRepositoryError, PersistMessageOutcome},
        transactions::TransactionHandle,
    },
};

type MessageRow = (
    Uuid,
    Uuid,
    Option<Uuid>,
    Option<Uuid>,
    Option<String>,
    OffsetDateTime,
);

pub(super) async fn persist(
    handle: &mut dyn TransactionHandle,
    command: &SendMessageCommand,
) -> Result<PersistMessageOutcome, MessagingRepositoryError> {
    let connection = connection(handle).map_err(|_| database_error("transaction_handle"))?;
    authorize(connection, command).await?;
    let proposed_id = Uuid::new_v4();
    let inserted = insert_message(connection, command, proposed_id).await?;
    let Some(row) = inserted else {
        return existing_message(connection, command).await;
    };
    let message = canonical_message(row);
    persist_event_and_outbox(connection, &message).await?;
    Ok(PersistMessageOutcome::Created(message))
}

async fn authorize(
    connection: &mut PgConnection,
    command: &SendMessageCommand,
) -> Result<(), MessagingRepositoryError> {
    let authorized = sqlx::query_scalar::<_, i32>(
        "SELECT 1 \
         FROM chatrooms c \
         JOIN groups g ON g.id = c.group_id AND g.deleted_at IS NULL \
         JOIN memberships m ON m.group_id = g.id AND m.user_id = $2 \
         WHERE c.id = $1 \
         FOR SHARE OF g, m",
    )
    .bind(command.chatroom_id)
    .bind(command.sender_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| database_error("authorize"))?;
    if authorized.is_none() {
        return Err(MessagingRepositoryError::MembershipRequired);
    }
    Ok(())
}

async fn insert_message(
    connection: &mut PgConnection,
    command: &SendMessageCommand,
    id: Uuid,
) -> Result<Option<MessageRow>, MessagingRepositoryError> {
    sqlx::query_as::<_, MessageRow>(
        "INSERT INTO messages (id, chatroom_id, sender_id, client_msg_id, body, type) \
         VALUES ($1, $2, $3, $4, $5, 'user') \
         ON CONFLICT (sender_id, client_msg_id) WHERE client_msg_id IS NOT NULL \
         DO NOTHING \
         RETURNING id, chatroom_id, sender_id, client_msg_id, body, created_at",
    )
    .bind(id)
    .bind(command.chatroom_id)
    .bind(command.sender_id)
    .bind(command.client_msg_id)
    .bind(&command.body)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| database_error("insert_message"))
}

async fn existing_message(
    connection: &mut PgConnection,
    command: &SendMessageCommand,
) -> Result<PersistMessageOutcome, MessagingRepositoryError> {
    let row = sqlx::query_as::<_, MessageRow>(
        "SELECT id, chatroom_id, sender_id, client_msg_id, body, created_at \
         FROM messages \
         WHERE sender_id = $1 AND client_msg_id = $2",
    )
    .bind(command.sender_id)
    .bind(command.client_msg_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| database_error("read_existing"))?
    .ok_or_else(|| database_error("missing_conflict_row"))?;
    if row.1 != command.chatroom_id || row.4 != command.body {
        return Err(MessagingRepositoryError::IdempotencyConflict);
    }
    Ok(PersistMessageOutcome::Existing(canonical_message(row)))
}

async fn persist_event_and_outbox(
    connection: &mut PgConnection,
    message: &CanonicalMessage,
) -> Result<(), MessagingRepositoryError> {
    let event_id = Uuid::new_v4();
    let payload = serde_json::to_value(message).map_err(|_| database_error("event_payload"))?;
    let (cursor, occurred_at) = sqlx::query_as::<_, (i64, OffsetDateTime)>(
        "INSERT INTO conversation_events \
         (id, conversation_id, event_type, event_version, payload) \
         VALUES ($1, $2, 'message.created', 1, $3) \
         RETURNING cursor, occurred_at",
    )
    .bind(event_id)
    .bind(message.chatroom_id)
    .bind(payload)
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| database_error("insert_event"))?;
    let event = MessageCreatedEvent {
        version: 1,
        event_type: MessageCreatedType::MessageCreated,
        event_id,
        conversation_id: message.chatroom_id,
        cursor: cursor.to_string(),
        occurred_at,
        data: message.clone(),
    };
    insert_outbox(connection, &event).await
}

async fn insert_outbox(
    connection: &mut PgConnection,
    event: &MessageCreatedEvent,
) -> Result<(), MessagingRepositoryError> {
    let payload = serde_json::to_value(event).map_err(|_| database_error("outbox_payload"))?;
    sqlx::query(
        "INSERT INTO outbox_events \
         (id, intent_type, event_type, event_version, aggregate_type, aggregate_id, \
          conversation_event_id, payload) \
         VALUES ($1, 'conversation', 'message.created', 1, 'conversation', $2, $3, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(event.conversation_id)
    .bind(event.event_id)
    .bind(payload)
    .execute(&mut *connection)
    .await
    .map(|_| ())
    .map_err(|_| database_error("insert_outbox"))
}

fn canonical_message(row: MessageRow) -> CanonicalMessage {
    CanonicalMessage {
        id: row.0,
        chatroom_id: row.1,
        sender_id: row.2,
        client_msg_id: row.3,
        body: row.4,
        message_type: MessageKind::User,
        created_at: row.5,
        media: Vec::new(),
    }
}

fn database_error(operation: &'static str) -> MessagingRepositoryError {
    tracing::warn!(
        dependency = "postgres",
        failure_kind = "messaging_write",
        operation,
        "PostgreSQL messaging write failed"
    );
    MessagingRepositoryError::DatabaseUnavailable
}
