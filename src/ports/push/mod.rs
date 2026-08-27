//! Notification history and durable Expo-push persistence boundaries.

use std::{collections::BTreeMap, fmt, future::Future, pin::Pin, time::Duration};

use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::ports::transactions::TransactionHandle;

pub type NotificationArgs = BTreeMap<String, Value>;

pub type NotificationsRepositoryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, NotificationsRepositoryError>> + Send + 'a>>;

pub trait NotificationsRepository: Send + Sync {
    fn list_notifications(
        &self,
        query: ListNotificationsQuery,
    ) -> NotificationsRepositoryFuture<'_, NotificationPage>;

    fn mark_notification_read<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a MarkNotificationReadCommand,
    ) -> NotificationsRepositoryFuture<'a, NotificationReadRecord>;
}

pub type NotificationEventsRepositoryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, NotificationsRepositoryError>> + Send + 'a>>;

pub trait NotificationEventsRepository: Send + Sync {
    fn record_topic_created<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a RecordTopicNotificationCommand,
    ) -> NotificationEventsRepositoryFuture<'a, NotificationFanoutReport>;

    fn record_message_created<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a RecordMessageNotificationCommand,
    ) -> NotificationEventsRepositoryFuture<'a, NotificationFanoutReport>;

    fn clear_topic_notifications<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a ClearTopicNotificationsCommand,
    ) -> NotificationEventsRepositoryFuture<'a, NotificationClearReport>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ListNotificationsQuery {
    pub user_id: Uuid,
    pub after: Option<Uuid>,
    pub limit: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MarkNotificationReadCommand {
    pub user_id: Uuid,
    pub notification_id: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordTopicNotificationCommand {
    pub group_id: Uuid,
    pub topic_id: Uuid,
    pub conversation_id: Uuid,
    pub source_event_id: Uuid,
    pub author_id: Uuid,
    pub author_display_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordMessageNotificationCommand {
    pub group_id: Uuid,
    pub topic_id: Uuid,
    pub conversation_id: Uuid,
    pub source_event_id: Uuid,
    pub source_message_id: Uuid,
    pub sender_id: Uuid,
    pub sender_display_name: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClearTopicNotificationsCommand {
    pub user_id: Uuid,
    pub conversation_id: Uuid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NotificationFanoutReport {
    pub notification_count: u64,
    pub occurrence_count: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NotificationClearReport {
    pub cleared_count: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationType {
    NewTopic,
    ChatUnread,
    Other,
}

impl NotificationType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NewTopic => "new_topic",
            Self::ChatUnread => "chat_unread",
            Self::Other => "other",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "new_topic" => Some(Self::NewTopic),
            "chat_unread" => Some(Self::ChatUnread),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationRecord {
    pub id: Uuid,
    pub notification_type: NotificationType,
    pub args: NotificationArgs,
    pub topic_id: Option<Uuid>,
    pub conversation_id: Option<Uuid>,
    pub source_cursor: Option<i64>,
    pub read_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationPage {
    pub items: Vec<NotificationRecord>,
    pub next_cursor: Option<String>,
    pub unread_count: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NotificationReadRecord {
    pub notification_id: Uuid,
    pub read_at: OffsetDateTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationsRepositoryError {
    NotificationNotFound,
    CursorInvalid,
    InvalidData,
    Unavailable,
}

impl fmt::Display for NotificationsRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("notification persistence operation failed")
    }
}

impl std::error::Error for NotificationsRepositoryError {}

pub type PushRepositoryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, PushRepositoryError>> + Send + 'a>>;

pub trait PushRepository: Send + Sync {
    fn upsert_installation<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a UpsertPushInstallationCommand,
    ) -> PushRepositoryFuture<'a, UpsertPushInstallationOutcome>;

    fn update_installation<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a UpdatePushInstallationCommand,
    ) -> PushRepositoryFuture<'a, PushInstallationRecord>;

    fn delete_installation<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a DeletePushInstallationCommand,
    ) -> PushRepositoryFuture<'a, ()>;
}

pub type PushSendAuthorizationFuture<'a> = Pin<
    Box<
        dyn Future<Output = Result<Option<AuthorizedPushDelivery>, PushRepositoryError>>
            + Send
            + 'a,
    >,
>;

pub trait PushSendAuthorizationRepository: Send + Sync {
    fn authorize_send<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        claim: &'a PushDeliveryClaim,
    ) -> PushSendAuthorizationFuture<'a>;
}

pub type PushPrivacyFenceFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), PushRepositoryError>> + Send + 'a>>;

pub trait PushPrivacyFence: Send + Sync {
    fn fence_membership_revocation<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a FenceMembershipPushCommand,
    ) -> PushPrivacyFenceFuture<'a>;

    fn fence_group_deletion<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a FenceGroupPushCommand,
    ) -> PushPrivacyFenceFuture<'a>;
}

pub type PushDeliveryRepositoryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, PushRepositoryError>> + Send + 'a>>;

pub trait PushDeliveryRepository: Send + Sync {
    fn claim_deliveries(
        &self,
        request: PushDeliveryClaimRequest,
    ) -> PushDeliveryRepositoryFuture<'_, Vec<ClaimedPushDelivery>>;

    fn mark_delivery_succeeded<'a>(
        &'a self,
        claim: &'a ClaimedPushDelivery,
    ) -> PushDeliveryRepositoryFuture<'a, bool>;

    fn record_delivery_failure<'a>(
        &'a self,
        claim: &'a ClaimedPushDelivery,
        code: PushDeliveryFailureCode,
        retry_delay: Duration,
        max_attempts: u32,
    ) -> PushDeliveryRepositoryFuture<'a, PushDeliveryFailureDisposition>;
}

pub type PushInvalidDestinationFuture<'a> =
    Pin<Box<dyn Future<Output = Result<bool, PushRepositoryError>> + Send + 'a>>;

pub trait PushInvalidDestinationRepository: Send + Sync {
    fn disable_invalid_destination<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        claim: &'a PushDeliveryClaim,
        destination: &'a ExpoPushDestination,
    ) -> PushInvalidDestinationFuture<'a>;
}

pub trait PushWorkerRepository:
    PushDeliveryRepository + PushSendAuthorizationRepository + PushInvalidDestinationRepository
{
}

impl<Repository> PushWorkerRepository for Repository where
    Repository:
        PushDeliveryRepository + PushSendAuthorizationRepository + PushInvalidDestinationRepository
{
}

pub type PushPreviewSourceFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<String>, PushRepositoryError>> + Send + 'a>>;

pub trait PushPreviewSource: Send + Sync {
    fn load_message_body(&self, message_id: Uuid) -> PushPreviewSourceFuture<'_>;
}

pub type PushProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<PushProviderOutcome, PushProviderError>> + Send + 'a>>;

pub trait PushProvider: Send + Sync {
    fn send<'a>(&'a self, request: &'a PushProviderRequest) -> PushProviderFuture<'a>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FenceMembershipPushCommand {
    pub group_id: Uuid,
    pub user_id: Uuid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FenceGroupPushCommand {
    pub group_id: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PushDeliveryClaim {
    pub occurrence_id: Uuid,
    pub claim_owner: String,
    pub claim_generation: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PushDeliveryClaimRequest {
    pub claim_owner: String,
    pub batch_size: u32,
    pub lease_duration: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimedPushDelivery {
    pub claim: PushDeliveryClaim,
    pub claim_expires_at: OffsetDateTime,
    pub attempt_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PushDeliveryFailureCode {
    AuthorizationDenied,
    PreviewUnavailable,
    ExpoRejected,
    ExpoUnavailable,
    ExpoTimeout,
}

impl PushDeliveryFailureCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AuthorizationDenied => "authorization_denied",
            Self::PreviewUnavailable => "preview_unavailable",
            Self::ExpoRejected => "expo_rejected",
            Self::ExpoUnavailable => "expo_unavailable",
            Self::ExpoTimeout => "expo_timeout",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PushDeliveryFailureDisposition {
    RetryScheduled,
    DeadLettered,
    StaleClaim,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizedPushDelivery {
    pub occurrence_id: Uuid,
    pub route: PushTapPayload,
    pub destination: ExpoPushDestination,
    pub preview_message_id: Option<Uuid>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PushTapPayload {
    pub notification_type: NotificationType,
    pub notification_id: Uuid,
    pub conversation_id: Uuid,
    pub message_id: Option<Uuid>,
}

#[derive(Clone, Eq, PartialEq)]
pub struct PushProviderRequest {
    pub destination: ExpoPushDestination,
    pub route: PushTapPayload,
    pub preview: Option<String>,
}

impl fmt::Debug for PushProviderRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PushProviderRequest")
            .field("destination", &self.destination)
            .field("route", &self.route)
            .field("preview", &self.preview.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PushProviderOutcome {
    Accepted,
    DeviceNotRegistered,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PushProviderError {
    Unavailable,
    Rejected,
}

impl fmt::Display for PushProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("push provider operation failed")
    }
}

impl std::error::Error for PushProviderError {}

#[derive(Clone, Eq, PartialEq)]
pub struct ExpoPushDestination {
    environment: PushEnvironment,
    token: String,
}

impl ExpoPushDestination {
    pub fn new(environment: PushEnvironment, token: String) -> Self {
        Self { environment, token }
    }

    pub fn environment(&self) -> PushEnvironment {
        self.environment
    }

    pub fn token(&self) -> &str {
        &self.token
    }
}

impl fmt::Debug for ExpoPushDestination {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExpoPushDestination")
            .field("environment", &self.environment)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PushPlatform {
    Ios,
    Android,
}

impl PushPlatform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ios => "ios",
            Self::Android => "android",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ios" => Some(Self::Ios),
            "android" => Some(Self::Android),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PushEnvironment {
    Development,
    Production,
}

impl PushEnvironment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Production => "production",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "development" => Some(Self::Development),
            "production" => Some(Self::Production),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PushProviderName {
    Expo,
}

impl PushProviderName {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Expo => "expo",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpsertPushInstallationCommand {
    pub id: Uuid,
    pub user_id: Uuid,
    pub installation_id: String,
    pub platform: PushPlatform,
    pub provider: PushProviderName,
    pub token: String,
    pub environment: PushEnvironment,
    pub message_preview_enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdatePushInstallationCommand {
    pub user_id: Uuid,
    pub installation_id: String,
    pub token: String,
    pub message_preview_enabled: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeletePushInstallationCommand {
    pub user_id: Uuid,
    pub installation_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PushInstallationRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub owner_epoch: i64,
    pub installation_id: String,
    pub platform: PushPlatform,
    pub provider: PushProviderName,
    pub token: String,
    pub environment: PushEnvironment,
    pub message_preview_enabled: bool,
    pub last_seen_at: OffsetDateTime,
    pub disabled_at: Option<OffsetDateTime>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpsertPushInstallationOutcome {
    pub installation: PushInstallationRecord,
    pub created: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PushRepositoryError {
    InstallationNotFound,
    InvalidData,
    Unavailable,
}

impl fmt::Display for PushRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("push persistence operation failed")
    }
}

impl std::error::Error for PushRepositoryError {}
