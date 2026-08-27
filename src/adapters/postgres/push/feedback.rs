use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::ports::push::{ExpoPushDestination, PushDeliveryClaim, PushRepositoryError};

use super::database_error;

const DEVICE_NOT_REGISTERED: &str = "device_not_registered";
const MAX_CLAIM_OWNER_CHARS: usize = 128;
const MAX_EXPO_TOKEN_CHARS: usize = 512;

#[derive(Clone, Copy)]
struct OccurrenceTopology {
    recipient_id: Uuid,
    installation_id: Uuid,
    installation_owner_epoch: i64,
}

pub(super) async fn load_message_body(
    pool: &PgPool,
    message_id: Uuid,
) -> Result<Option<String>, PushRepositoryError> {
    sqlx::query_scalar::<_, Option<String>>("SELECT body FROM messages WHERE id = $1")
        .bind(message_id)
        .fetch_optional(pool)
        .await
        .map(Option::flatten)
        .map_err(|error| database_error("push_preview_message_body", error))
}

pub(super) async fn disable_invalid_destination(
    connection: &mut PgConnection,
    claim: &PushDeliveryClaim,
    destination: &ExpoPushDestination,
) -> Result<bool, PushRepositoryError> {
    validate_claim(claim)?;
    validate_destination(destination)?;
    let Some(topology) = occurrence_topology(connection, claim.occurrence_id).await? else {
        return Ok(false);
    };
    if !lock_exact_installation(connection, &topology, destination).await? {
        return Ok(false);
    }
    if !terminalize_live_claim(connection, claim, &topology).await? {
        return Ok(false);
    }
    disable_installation(connection, &topology, destination).await?;
    Ok(true)
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

fn validate_destination(destination: &ExpoPushDestination) -> Result<(), PushRepositoryError> {
    let token = destination.token();
    let token_length = token.chars().count();
    if token_length == 0
        || token_length > MAX_EXPO_TOKEN_CHARS
        || token.trim() != token
        || token.chars().any(char::is_control)
    {
        return Err(PushRepositoryError::InvalidData);
    }
    Ok(())
}

async fn occurrence_topology(
    connection: &mut PgConnection,
    occurrence_id: Uuid,
) -> Result<Option<OccurrenceTopology>, PushRepositoryError> {
    let row = sqlx::query_as::<_, (Uuid, Uuid, i64)>(
        "SELECT recipient_user_id, push_installation_id, installation_owner_epoch \
         FROM push_delivery_intents WHERE id = $1",
    )
    .bind(occurrence_id)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("push_invalid_destination_topology", error))?;
    row.map(
        |(recipient_id, installation_id, installation_owner_epoch)| {
            if installation_owner_epoch <= 0 {
                return Err(PushRepositoryError::InvalidData);
            }
            Ok(OccurrenceTopology {
                recipient_id,
                installation_id,
                installation_owner_epoch,
            })
        },
    )
    .transpose()
}

async fn lock_exact_installation(
    connection: &mut PgConnection,
    topology: &OccurrenceTopology,
    destination: &ExpoPushDestination,
) -> Result<bool, PushRepositoryError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM push_installations \
         WHERE id = $1 AND user_id = $2 AND owner_epoch = $3 \
           AND provider = 'expo' AND environment = $4 AND token = $5 \
           AND disabled_at IS NULL \
         FOR UPDATE",
    )
    .bind(topology.installation_id)
    .bind(topology.recipient_id)
    .bind(topology.installation_owner_epoch)
    .bind(destination.environment().as_str())
    .bind(destination.token())
    .fetch_optional(connection)
    .await
    .map(|row| row.is_some())
    .map_err(|error| database_error("push_invalid_destination_installation_lock", error))
}

async fn terminalize_live_claim(
    connection: &mut PgConnection,
    claim: &PushDeliveryClaim,
    topology: &OccurrenceTopology,
) -> Result<bool, PushRepositoryError> {
    let result = sqlx::query(
        "WITH server_clock AS MATERIALIZED ( \
             SELECT clock_timestamp() AS now \
         ) \
         UPDATE push_delivery_intents \
         SET status = 'failed', attempt_count = attempt_count + 1, \
             claim_owner = NULL, lease_expires_at = NULL, next_attempt_at = NULL, \
             last_error_code = $7, failed_at = (SELECT now FROM server_clock) \
         WHERE id = $1 AND recipient_user_id = $2 AND push_installation_id = $3 \
           AND installation_owner_epoch = $4 AND provider = 'expo' \
           AND status = 'claimed' AND claim_owner = $5 AND claim_generation = $6 \
           AND lease_expires_at > (SELECT now FROM server_clock)",
    )
    .bind(claim.occurrence_id)
    .bind(topology.recipient_id)
    .bind(topology.installation_id)
    .bind(topology.installation_owner_epoch)
    .bind(&claim.claim_owner)
    .bind(claim.claim_generation)
    .bind(DEVICE_NOT_REGISTERED)
    .execute(connection)
    .await
    .map_err(|error| database_error("push_invalid_destination_occurrence", error))?;
    Ok(result.rows_affected() == 1)
}

async fn disable_installation(
    connection: &mut PgConnection,
    topology: &OccurrenceTopology,
    destination: &ExpoPushDestination,
) -> Result<(), PushRepositoryError> {
    let result = sqlx::query(
        "UPDATE push_installations SET disabled_at = clock_timestamp() \
         WHERE id = $1 AND user_id = $2 AND owner_epoch = $3 \
           AND provider = 'expo' AND environment = $4 AND token = $5 \
           AND disabled_at IS NULL",
    )
    .bind(topology.installation_id)
    .bind(topology.recipient_id)
    .bind(topology.installation_owner_epoch)
    .bind(destination.environment().as_str())
    .bind(destination.token())
    .execute(connection)
    .await
    .map_err(|error| database_error("push_invalid_destination_installation", error))?;
    if result.rows_affected() != 1 {
        return Err(PushRepositoryError::InvalidData);
    }
    Ok(())
}
