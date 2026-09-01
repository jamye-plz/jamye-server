//! Reliable message command and delta recovery use cases.

use std::{error::Error, fmt, sync::Arc};

use uuid::Uuid;

use crate::{
    application::auth::AccessIdentity,
    domain::messaging::{CanonicalMessage, EventPage, SendMessageCommand},
    ports::{
        messaging::{
            ContractProjection, DeltaQuery, MessageDeliveryContext, MessagingRepository,
            MessagingRepositoryError, PersistMessageOutcome,
        },
        transactions::{BoxTransactionHandle, TransactionHandle, TransactionManager},
    },
};

pub const CURRENT_CONTRACT_VERSION: &str = "1";
pub const PREVIOUS_CONTRACT_VERSION: &str = "0";
pub const DEFAULT_DELTA_LIMIT: u32 = 50;
pub const MAX_DELTA_LIMIT: u32 = 100;

#[derive(Clone)]
pub struct MessagingService {
    transactions: Arc<dyn TransactionManager>,
    repository: Arc<dyn MessagingRepository>,
}

impl MessagingService {
    pub fn new(
        transactions: Arc<dyn TransactionManager>,
        repository: Arc<dyn MessagingRepository>,
    ) -> Self {
        Self {
            transactions,
            repository,
        }
    }

    pub async fn send_message(
        &self,
        identity: &AccessIdentity,
        input: SendMessageInput,
    ) -> Result<SendMessageOutcome, MessagingError> {
        validate_message(&input)?;
        let command = SendMessageCommand {
            chatroom_id: input.chatroom_id,
            sender_id: identity.user_id,
            client_msg_id: input.client_msg_id,
            body: input.body,
        };
        let mut handle = self
            .transactions
            .begin()
            .await
            .map_err(|_| MessagingError::DatabaseUnavailable)?;
        let result = self
            .send_command_in_transaction(handle.as_mut(), &command)
            .await;
        self.finish_send(handle, result).await
    }

    /// Persists an already-constructed messaging command on a caller-owned
    /// Task-4a transaction.  The caller owns the transaction outcome.
    pub(crate) async fn send_command_in_transaction(
        &self,
        transaction: &mut dyn TransactionHandle,
        command: &SendMessageCommand,
    ) -> Result<PersistMessageOutcome, MessagingError> {
        self.repository
            .send(transaction, command)
            .await
            .map_err(MessagingError::from)
    }

    /// Validates the mounted HTTP input and creates the feature command before
    /// a caller decides whether opening a transaction is appropriate.
    ///
    /// This bridge deliberately permits exactly one media upload.  The
    /// standalone messaging wrapper retains its legacy media-unavailable
    /// projection and its own begin/commit/rollback lifecycle.
    pub(crate) fn prepare_http_send(
        &self,
        actor_id: Uuid,
        input: &SendMessageInput,
    ) -> Result<SendMessageCommand, MessagingError> {
        validate_composed_http_message(input)?;
        Ok(SendMessageCommand {
            chatroom_id: input.chatroom_id,
            sender_id: actor_id,
            client_msg_id: input.client_msg_id,
            body: input.body.clone(),
        })
    }

    /// Persists a validated mounted-HTTP command and obtains the authoritative
    /// delivery context on the caller-owned transaction.
    pub(crate) async fn send_http_command_in_transaction(
        &self,
        transaction: &mut dyn TransactionHandle,
        command: &SendMessageCommand,
    ) -> Result<ComposedMessageSend, MessagingError> {
        let outcome = self
            .send_command_in_transaction(transaction, command)
            .await?;
        let persisted = match &outcome {
            PersistMessageOutcome::Created(message) | PersistMessageOutcome::Existing(message) => {
                message
            }
        };
        let delivery_context = self
            .repository
            .delivery_context(transaction, persisted)
            .await
            .map_err(MessagingError::from)?;
        Ok(ComposedMessageSend {
            outcome,
            delivery_context,
        })
    }

    pub async fn events(
        &self,
        identity: &AccessIdentity,
        input: DeltaInput,
    ) -> Result<EventPage, MessagingError> {
        let projection = match input.contract_version.as_str() {
            CURRENT_CONTRACT_VERSION => ContractProjection::Current,
            PREVIOUS_CONTRACT_VERSION => ContractProjection::Previous,
            _ => return Err(MessagingError::ContractUpgradeRequired),
        };
        self.repository
            .events(DeltaQuery {
                conversation_id: input.conversation_id,
                user_id: identity.user_id,
                after: input.after,
                limit: input.limit,
                projection,
            })
            .await
            .map_err(MessagingError::from)
    }

    async fn finish_send(
        &self,
        handle: BoxTransactionHandle,
        result: Result<PersistMessageOutcome, MessagingError>,
    ) -> Result<SendMessageOutcome, MessagingError> {
        match result {
            Ok(outcome) => {
                self.transactions
                    .commit(handle)
                    .await
                    .map_err(|_| MessagingError::DatabaseUnavailable)?;
                Ok(outcome.into())
            }
            Err(error) => {
                self.transactions
                    .rollback(handle)
                    .await
                    .map_err(|_| MessagingError::DatabaseUnavailable)?;
                Err(error)
            }
        }
    }
}

/// Feature-owned data retained by Task-12 while it appends media and
/// notification operations to the same transaction.
pub(crate) struct ComposedMessageSend {
    pub outcome: PersistMessageOutcome,
    pub delivery_context: MessageDeliveryContext,
}

fn validate_message(input: &SendMessageInput) -> Result<(), MessagingError> {
    validate_message_base(input)?;
    if !input.media_upload_ids.is_empty() {
        return Err(MessagingError::MediaNotAvailable);
    }
    Ok(())
}

fn validate_composed_http_message(input: &SendMessageInput) -> Result<(), MessagingError> {
    validate_message_base(input)?;
    if input.media_upload_ids.len() > 1 {
        return Err(MessagingError::MediaNotAvailable);
    }
    Ok(())
}

fn validate_message_base(input: &SendMessageInput) -> Result<(), MessagingError> {
    if input
        .idempotency_key
        .is_some_and(|header| header != input.client_msg_id)
    {
        return Err(MessagingError::IdempotencyKeyMismatch);
    }
    let body_present = input.body.as_ref().is_some_and(|body| !body.is_empty());
    if !body_present && input.media_upload_ids.is_empty() {
        return Err(MessagingError::MessageContentRequired);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SendMessageInput {
    pub chatroom_id: Uuid,
    pub client_msg_id: Uuid,
    pub body: Option<String>,
    pub media_upload_ids: Vec<Uuid>,
    pub idempotency_key: Option<Uuid>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeltaInput {
    pub conversation_id: Uuid,
    pub after: Option<i64>,
    pub limit: u32,
    pub contract_version: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SendMessageOutcome {
    Created(CanonicalMessage),
    Existing(CanonicalMessage),
}

impl From<PersistMessageOutcome> for SendMessageOutcome {
    fn from(outcome: PersistMessageOutcome) -> Self {
        match outcome {
            PersistMessageOutcome::Created(message) => Self::Created(message.into_message()),
            PersistMessageOutcome::Existing(message) => Self::Existing(message.into_message()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessagingError {
    RequestValidation,
    MessageContentRequired,
    IdempotencyKeyMismatch,
    MediaNotAvailable,
    MembershipRequired,
    IdempotencyConflict,
    ContractUpgradeRequired,
    DatabaseUnavailable,
}

impl From<MessagingRepositoryError> for MessagingError {
    fn from(error: MessagingRepositoryError) -> Self {
        match error {
            MessagingRepositoryError::MembershipRequired => Self::MembershipRequired,
            MessagingRepositoryError::IdempotencyConflict => Self::IdempotencyConflict,
            MessagingRepositoryError::ContractUpgradeRequired => Self::ContractUpgradeRequired,
            MessagingRepositoryError::DatabaseUnavailable => Self::DatabaseUnavailable,
        }
    }
}

impl fmt::Display for MessagingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("messaging request failed")
    }
}

impl Error for MessagingError {}
