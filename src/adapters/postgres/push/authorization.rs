use sqlx::PgConnection;
use uuid::Uuid;

use crate::ports::push::{
    AuthorizedPushDelivery, ExpoPushDestination, NotificationType, PushDeliveryClaim,
    PushEnvironment, PushProviderName, PushRepositoryError, PushTapPayload,
};

use super::database_error;

const MAX_CLAIM_OWNER_CHARS: usize = 128;

#[derive(Clone, Copy)]
struct DeliveryTopology {
    group_id: Uuid,
    recipient_id: Uuid,
    notification_id: Uuid,
    installation_id: Uuid,
    conversation_id: Uuid,
}

struct LockedNotification {
    id: Uuid,
    notification_type: NotificationType,
    conversation_id: Uuid,
}

struct LockedInstallation {
    owner_epoch: i64,
    destination: ExpoPushDestination,
    message_preview_enabled: bool,
}

#[derive(Clone, Copy)]
struct LockedOccurrence {
    source_message_id: Option<Uuid>,
    message_preview_enabled_snapshot: bool,
}

pub(super) async fn authorize_send(
    connection: &mut PgConnection,
    claim: &PushDeliveryClaim,
) -> Result<Option<AuthorizedPushDelivery>, PushRepositoryError> {
    validate_claim(claim)?;
    let Some(topology) = delivery_topology(connection, claim.occurrence_id).await? else {
        return Ok(None);
    };
    if !lock_live_group(connection, topology.group_id).await?
        || !lock_membership(connection, topology.group_id, topology.recipient_id).await?
    {
        return Ok(None);
    }
    let Some(notification) = lock_notification(connection, &topology).await? else {
        return Ok(None);
    };
    let Some(installation) = lock_installation(connection, &topology).await? else {
        return Ok(None);
    };
    let Some(occurrence) = lock_occurrence(connection, claim, &topology, &installation).await?
    else {
        return Ok(None);
    };
    authorized_delivery(claim, notification, installation, occurrence).map(Some)
}

fn validate_claim(claim: &PushDeliveryClaim) -> Result<(), PushRepositoryError> {
    let owner_length = claim.claim_owner.chars().count();
    if claim.claim_generation <= 0
        || owner_length == 0
        || owner_length > MAX_CLAIM_OWNER_CHARS
        || claim.claim_owner.chars().any(char::is_control)
    {
        return Err(PushRepositoryError::InvalidData);
    }
    Ok(())
}

async fn delivery_topology(
    connection: &mut PgConnection,
    occurrence_id: Uuid,
) -> Result<Option<DeliveryTopology>, PushRepositoryError> {
    let row = sqlx::query_as::<_, (Uuid, Uuid, Uuid, Uuid, Uuid)>(
        "SELECT conversation.group_id, intent.recipient_user_id, intent.notification_id, \
                intent.push_installation_id, conversation.id \
         FROM push_delivery_intents intent \
         JOIN notifications notification ON notification.id = intent.notification_id \
         JOIN chatrooms conversation ON conversation.id = notification.conversation_id \
         WHERE intent.id = $1",
    )
    .bind(occurrence_id)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("push_send_topology", error))?;
    Ok(row.map(
        |(group_id, recipient_id, notification_id, installation_id, conversation_id)| {
            DeliveryTopology {
                group_id,
                recipient_id,
                notification_id,
                installation_id,
                conversation_id,
            }
        },
    ))
}

async fn lock_live_group(
    connection: &mut PgConnection,
    group_id: Uuid,
) -> Result<bool, PushRepositoryError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM groups WHERE id = $1 AND deleted_at IS NULL FOR SHARE",
    )
    .bind(group_id)
    .fetch_optional(connection)
    .await
    .map(|row| row.is_some())
    .map_err(|error| database_error("push_send_group_lock", error))
}

async fn lock_membership(
    connection: &mut PgConnection,
    group_id: Uuid,
    recipient_id: Uuid,
) -> Result<bool, PushRepositoryError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM memberships \
         WHERE group_id = $1 AND user_id = $2 \
         FOR SHARE",
    )
    .bind(group_id)
    .bind(recipient_id)
    .fetch_optional(connection)
    .await
    .map(|row| row.is_some())
    .map_err(|error| database_error("push_send_membership_lock", error))
}

async fn lock_notification(
    connection: &mut PgConnection,
    topology: &DeliveryTopology,
) -> Result<Option<LockedNotification>, PushRepositoryError> {
    let row = sqlx::query_as::<_, (Uuid, String, Option<Uuid>)>(
        "SELECT user_id, type, conversation_id \
         FROM notifications WHERE id = $1 FOR SHARE",
    )
    .bind(topology.notification_id)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("push_send_notification_lock", error))?;
    let Some((user_id, notification_type, conversation_id)) = row else {
        return Ok(None);
    };
    if user_id != topology.recipient_id || conversation_id != Some(topology.conversation_id) {
        return Ok(None);
    }
    let notification_type =
        NotificationType::parse(&notification_type).ok_or(PushRepositoryError::InvalidData)?;
    Ok(Some(LockedNotification {
        id: topology.notification_id,
        notification_type,
        conversation_id: topology.conversation_id,
    }))
}

async fn lock_installation(
    connection: &mut PgConnection,
    topology: &DeliveryTopology,
) -> Result<Option<LockedInstallation>, PushRepositoryError> {
    let row = sqlx::query_as::<_, (Uuid, i64, String, String, String, bool, bool)>(
        "SELECT user_id, owner_epoch, provider, token, environment, \
                message_preview_enabled, disabled_at IS NULL \
         FROM push_installations WHERE id = $1 FOR SHARE",
    )
    .bind(topology.installation_id)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("push_send_installation_lock", error))?;
    let Some((user_id, owner_epoch, provider, token, environment, preview, enabled)) = row else {
        return Ok(None);
    };
    if user_id != topology.recipient_id || !enabled {
        return Ok(None);
    }
    if provider != PushProviderName::Expo.as_str() || owner_epoch <= 0 {
        return Err(PushRepositoryError::InvalidData);
    }
    let environment =
        PushEnvironment::parse(&environment).ok_or(PushRepositoryError::InvalidData)?;
    Ok(Some(LockedInstallation {
        owner_epoch,
        destination: ExpoPushDestination::new(environment, token),
        message_preview_enabled: preview,
    }))
}

async fn lock_occurrence(
    connection: &mut PgConnection,
    claim: &PushDeliveryClaim,
    topology: &DeliveryTopology,
    installation: &LockedInstallation,
) -> Result<Option<LockedOccurrence>, PushRepositoryError> {
    let row = sqlx::query_as::<_, (Option<Uuid>, bool)>(
        "SELECT source_message_id, message_preview_enabled_snapshot \
         FROM push_delivery_intents \
         WHERE id = $1 AND notification_id = $2 AND recipient_user_id = $3 \
           AND push_installation_id = $4 AND installation_owner_epoch = $5 \
           AND provider = 'expo' AND status = 'claimed' AND claim_owner = $6 \
           AND claim_generation = $7 AND lease_expires_at > clock_timestamp() \
         FOR UPDATE",
    )
    .bind(claim.occurrence_id)
    .bind(topology.notification_id)
    .bind(topology.recipient_id)
    .bind(topology.installation_id)
    .bind(installation.owner_epoch)
    .bind(&claim.claim_owner)
    .bind(claim.claim_generation)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("push_send_occurrence_lock", error))?;
    Ok(row.map(
        |(source_message_id, message_preview_enabled_snapshot)| LockedOccurrence {
            source_message_id,
            message_preview_enabled_snapshot,
        },
    ))
}

fn authorized_delivery(
    claim: &PushDeliveryClaim,
    notification: LockedNotification,
    installation: LockedInstallation,
    occurrence: LockedOccurrence,
) -> Result<AuthorizedPushDelivery, PushRepositoryError> {
    validate_route_shape(notification.notification_type, occurrence.source_message_id)?;
    let preview_message_id = (installation.message_preview_enabled
        && occurrence.message_preview_enabled_snapshot)
        .then_some(occurrence.source_message_id)
        .flatten();
    Ok(AuthorizedPushDelivery {
        occurrence_id: claim.occurrence_id,
        route: PushTapPayload {
            notification_type: notification.notification_type,
            notification_id: notification.id,
            conversation_id: notification.conversation_id,
            message_id: occurrence.source_message_id,
        },
        destination: installation.destination,
        preview_message_id,
    })
}

fn validate_route_shape(
    notification_type: NotificationType,
    source_message_id: Option<Uuid>,
) -> Result<(), PushRepositoryError> {
    match (notification_type, source_message_id) {
        (NotificationType::NewTopic, None)
        | (NotificationType::ChatUnread, Some(_))
        | (NotificationType::Other, _) => Ok(()),
        _ => Err(PushRepositoryError::InvalidData),
    }
}
