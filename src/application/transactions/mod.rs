//! Task-12 static cross-feature transaction compositions.
//!
//! Sprint-2 composes existing feature operations with Task-4a's caller-owned
//! transaction manager. The dependency bundle is not a second transaction,
//! UnitOfWork, registry, or callback abstraction.

use std::{error::Error, fmt, sync::Arc};

use crate::{
    application::{
        chatrooms::{ChatroomsError, ChatroomsService},
        messaging::{MessagingError, MessagingService, SendMessageInput, SendMessageOutcome},
        topics::{TopicCreateInput, TopicsError, TopicsService},
    },
    domain::messaging::SendMessageCommand,
    ports::{
        chatrooms::MarkReadCommand,
        media::{
            AuthoritativeMessageMediaCommand, BindMessageMediaCommand, BindMessageMediaItem,
            MediaRepository, MediaRepositoryError,
        },
        messaging::{MessageDeliveryContext, PersistedMessage},
        push::{
            ClearTopicNotificationsCommand, NotificationEventsRepository,
            RecordMessageNotificationCommand, RecordTopicNotificationCommand,
        },
        topics::CreateTopicCommand,
        transactions::{BoxTransactionHandle, TransactionManager},
    },
};

#[derive(Clone)]
pub struct TransactionCompositions {
    dependencies: TransactionCompositionDependencies,
}

#[derive(Clone)]
pub struct TransactionCompositionDependencies {
    pub transactions: Arc<dyn TransactionManager>,
    pub messaging: Arc<MessagingService>,
    pub media: Arc<dyn MediaRepository>,
    pub topics: Arc<TopicsService>,
    pub chatrooms: Arc<ChatroomsService>,
    pub notifications: Arc<dyn NotificationEventsRepository>,
}

impl TransactionCompositions {
    pub fn new(dependencies: TransactionCompositionDependencies) -> Self {
        Self { dependencies }
    }

    pub async fn send_message(
        &self,
        input: SendMessageCompositionInput,
    ) -> Result<(), TransactionCompositionError> {
        let mut transaction = self.begin().await?;
        let result = async {
            let persisted = self
                .dependencies
                .messaging
                .send_command_in_transaction(transaction.as_mut(), &input.message)
                .await
                .map_err(|_| TransactionCompositionError::FeatureOperationFailed)?
                .into_persisted();
            let notification = input.notification_for(&persisted)?;
            let media = BindMessageMediaCommand {
                actor_id: input.message.sender_id,
                chatroom_id: persisted.message().chatroom_id,
                message_id: persisted.message().id,
                media: input.media,
            };
            self.dependencies
                .media
                .bind_message_media(transaction.as_mut(), &media)
                .await
                .map_err(|_| TransactionCompositionError::FeatureOperationFailed)?;
            self.dependencies
                .notifications
                .record_message_created(transaction.as_mut(), &notification)
                .await
                .map_err(|_| TransactionCompositionError::FeatureOperationFailed)?;
            Ok(())
        }
        .await;
        finish_feature(&self.dependencies.transactions, transaction, result).await
    }

    pub async fn create_topic(
        &self,
        input: CreateTopicCompositionInput,
    ) -> Result<(), TransactionCompositionError> {
        let mut transaction = self.begin().await?;
        let result = async {
            self.dependencies
                .topics
                .create_topic_command_in_transaction(transaction.as_mut(), &input.topic)
                .await
                .map_err(|_| TransactionCompositionError::FeatureOperationFailed)?;
            let notification = input.notification_for();
            self.dependencies
                .notifications
                .record_topic_created(transaction.as_mut(), &notification)
                .await
                .map_err(|_| TransactionCompositionError::FeatureOperationFailed)?;
            Ok(())
        }
        .await;
        finish_feature(&self.dependencies.transactions, transaction, result).await
    }

    pub async fn mark_conversation_read(
        &self,
        input: MarkConversationReadCompositionInput,
    ) -> Result<(), TransactionCompositionError> {
        let mut transaction = self.begin().await?;
        let result = async {
            self.dependencies
                .chatrooms
                .mark_read_command_in_transaction(transaction.as_mut(), &input.read)
                .await
                .map_err(|_| TransactionCompositionError::FeatureOperationFailed)?;
            let clear = input.notification_clear();
            self.dependencies
                .notifications
                .clear_topic_notifications(transaction.as_mut(), &clear)
                .await
                .map_err(|_| TransactionCompositionError::FeatureOperationFailed)?;
            Ok(())
        }
        .await;
        finish_feature(&self.dependencies.transactions, transaction, result).await
    }

    pub async fn send_message_http(
        &self,
        actor_id: uuid::Uuid,
        input: SendMessageInput,
    ) -> Result<SendMessageOutcome, MessagingError> {
        let command = self
            .dependencies
            .messaging
            .prepare_http_send(actor_id, &input)?;
        let mut transaction = self
            .begin()
            .await
            .map_err(|_| MessagingError::DatabaseUnavailable)?;
        let result = async {
            let composed = self
                .dependencies
                .messaging
                .send_http_command_in_transaction(transaction.as_mut(), &command)
                .await?;
            let outcome = composed.outcome;
            let created = matches!(
                &outcome,
                crate::ports::messaging::PersistMessageOutcome::Created(_)
            );
            let persisted = outcome.into_persisted();
            let attachments = self
                .dependencies
                .media
                .bind_authoritative_message_media(
                    transaction.as_mut(),
                    &AuthoritativeMessageMediaCommand {
                        actor_id,
                        chatroom_id: command.chatroom_id,
                        message_id: persisted.message().id,
                        upload_ids: input.media_upload_ids,
                    },
                )
                .await
                .map_err(map_media_error)?;
            match composed.delivery_context {
                MessageDeliveryContext::Main => {}
                MessageDeliveryContext::Topic {
                    group_id,
                    topic_id,
                    sender_display_name,
                } => {
                    self.dependencies
                        .notifications
                        .record_message_created(
                            transaction.as_mut(),
                            &RecordMessageNotificationCommand {
                                group_id,
                                topic_id,
                                conversation_id: command.chatroom_id,
                                source_event_id: persisted.source_event_id(),
                                source_message_id: persisted.message().id,
                                sender_id: actor_id,
                                sender_display_name,
                            },
                        )
                        .await
                        .map_err(|_| MessagingError::DatabaseUnavailable)?;
                }
            }
            let mut message = persisted.into_message();
            message.media = attachments;
            Ok::<_, MessagingError>(if created {
                SendMessageOutcome::Created(message)
            } else {
                SendMessageOutcome::Existing(message)
            })
        }
        .await;
        finish_feature(&self.dependencies.transactions, transaction, result).await
    }

    pub async fn create_topic_http(
        &self,
        author_id: uuid::Uuid,
        group_id: uuid::Uuid,
        input: TopicCreateInput,
    ) -> Result<crate::ports::topics::CreateTopicOutcome, TopicsError> {
        let command = self
            .dependencies
            .topics
            .prepare_create_topic(author_id, group_id, input)?;
        let mut transaction = self
            .begin()
            .await
            .map_err(|_| TopicsError::DatabaseUnavailable)?;
        let result = async {
            let (outcome, context) = self
                .dependencies
                .topics
                .create_topic_command_with_notification_context_in_transaction(
                    transaction.as_mut(),
                    &command,
                )
                .await?;
            self.dependencies
                .notifications
                .record_topic_created(
                    transaction.as_mut(),
                    &RecordTopicNotificationCommand {
                        group_id: context.group_id,
                        topic_id: context.topic_id,
                        conversation_id: context.conversation_id,
                        source_event_id: context.source_event_id,
                        author_id: context.author_id,
                        author_display_name: context.author_display_name,
                    },
                )
                .await
                .map_err(|_| TopicsError::DatabaseUnavailable)?;
            Ok::<_, TopicsError>(outcome)
        }
        .await;
        finish_feature(&self.dependencies.transactions, transaction, result).await
    }

    pub async fn mark_read_http(
        &self,
        user_id: uuid::Uuid,
        chatroom_id: uuid::Uuid,
        cursor: i64,
    ) -> Result<crate::ports::chatrooms::ReadMarker, ChatroomsError> {
        let command =
            self.dependencies
                .chatrooms
                .prepare_mark_read_command(user_id, chatroom_id, cursor)?;
        let mut transaction = self
            .begin()
            .await
            .map_err(|_| ChatroomsError::DatabaseUnavailable)?;
        let result = async {
            let marker = self
                .dependencies
                .chatrooms
                .mark_read_command_in_transaction(transaction.as_mut(), &command)
                .await?;
            self.dependencies
                .notifications
                .clear_topic_notifications(
                    transaction.as_mut(),
                    &ClearTopicNotificationsCommand {
                        user_id,
                        conversation_id: chatroom_id,
                    },
                )
                .await
                .map_err(|_| ChatroomsError::DatabaseUnavailable)?;
            Ok::<_, ChatroomsError>(marker)
        }
        .await;
        finish_feature(&self.dependencies.transactions, transaction, result).await
    }

    async fn begin(&self) -> Result<BoxTransactionHandle, TransactionCompositionError> {
        self.dependencies
            .transactions
            .begin()
            .await
            .map_err(|_| TransactionCompositionError::DatabaseUnavailable)
    }
}

async fn finish_feature<T, E>(
    transactions: &Arc<dyn TransactionManager>,
    transaction: BoxTransactionHandle,
    result: Result<T, E>,
) -> Result<T, E>
where
    E: From<TransactionCompositionError>,
{
    match result {
        Ok(value) => transactions
            .commit(transaction)
            .await
            .map_err(|_| E::from(TransactionCompositionError::DatabaseUnavailable))
            .map(|_| value),
        Err(error) => {
            transactions
                .rollback(transaction)
                .await
                .map_err(|_| E::from(TransactionCompositionError::DatabaseUnavailable))?;
            Err(error)
        }
    }
}

fn map_media_error(error: MediaRepositoryError) -> MessagingError {
    match error {
        MediaRepositoryError::Unavailable => MessagingError::DatabaseUnavailable,
        _ => MessagingError::MediaNotAvailable,
    }
}

impl From<TransactionCompositionError> for MessagingError {
    fn from(_: TransactionCompositionError) -> Self {
        Self::DatabaseUnavailable
    }
}
impl From<TransactionCompositionError> for TopicsError {
    fn from(_: TransactionCompositionError) -> Self {
        Self::DatabaseUnavailable
    }
}
impl From<TransactionCompositionError> for ChatroomsError {
    fn from(_: TransactionCompositionError) -> Self {
        Self::DatabaseUnavailable
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SendMessageCompositionInput {
    pub message: SendMessageCommand,
    pub group_id: uuid::Uuid,
    pub topic_id: uuid::Uuid,
    pub sender_display_name: String,
    pub media: Vec<BindMessageMediaItem>,
}

impl SendMessageCompositionInput {
    pub fn notification_for(
        &self,
        persisted: &PersistedMessage,
    ) -> Result<RecordMessageNotificationCommand, TransactionCompositionError> {
        let message = persisted.message();
        if message.chatroom_id != self.message.chatroom_id
            || message.sender_id != Some(self.message.sender_id)
        {
            return Err(TransactionCompositionError::IdentifierMismatch);
        }
        Ok(RecordMessageNotificationCommand {
            group_id: self.group_id,
            topic_id: self.topic_id,
            conversation_id: message.chatroom_id,
            source_event_id: persisted.source_event_id(),
            source_message_id: message.id,
            sender_id: self.message.sender_id,
            sender_display_name: self.sender_display_name.clone(),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreateTopicCompositionInput {
    pub topic: CreateTopicCommand,
    pub author_display_name: String,
}

impl CreateTopicCompositionInput {
    pub fn notification_for(&self) -> RecordTopicNotificationCommand {
        RecordTopicNotificationCommand {
            group_id: self.topic.group_id,
            topic_id: self.topic.topic_id,
            conversation_id: self.topic.topic_chatroom_id,
            source_event_id: self.topic.topic_event_id,
            author_id: self.topic.author_id,
            author_display_name: self.author_display_name.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MarkConversationReadCompositionInput {
    pub read: MarkReadCommand,
}

impl MarkConversationReadCompositionInput {
    pub fn notification_clear(&self) -> ClearTopicNotificationsCommand {
        ClearTopicNotificationsCommand {
            user_id: self.read.user_id,
            conversation_id: self.read.chatroom_id,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionCompositionError {
    DatabaseUnavailable,
    FeatureOperationFailed,
    IdentifierMismatch,
}

impl fmt::Display for TransactionCompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("transaction composition failed")
    }
}

impl Error for TransactionCompositionError {}
