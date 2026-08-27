use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::ports::push::{
    DeletePushInstallationCommand, PushEnvironment, PushInstallationRecord, PushPlatform,
    PushProviderName, PushRepositoryError, UpdatePushInstallationCommand,
    UpsertPushInstallationCommand, UpsertPushInstallationOutcome,
};

use super::database_error;

type InstallationRow = (
    Uuid,
    Uuid,
    i64,
    String,
    String,
    String,
    String,
    String,
    bool,
    OffsetDateTime,
    Option<OffsetDateTime>,
);

pub(super) async fn upsert_installation(
    connection: &mut PgConnection,
    command: &UpsertPushInstallationCommand,
) -> Result<UpsertPushInstallationOutcome, PushRepositoryError> {
    lock_identity(connection, &command.installation_id).await?;
    lock_destination(connection, &command.token).await?;

    let rows = sqlx::query_as::<_, InstallationRow>(
        "SELECT id, user_id, owner_epoch, installation_id, platform, provider, token, \
                environment, message_preview_enabled, last_seen_at, disabled_at \
         FROM push_installations \
         WHERE installation_id = $1 OR (environment = $2 AND token = $3) \
         ORDER BY id \
         FOR UPDATE",
    )
    .bind(&command.installation_id)
    .bind(command.environment.as_str())
    .bind(&command.token)
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| database_error("push_installation_upsert_lock", error))?;

    let identity_index = rows.iter().position(|row| row.3 == command.installation_id);
    let destination_index = rows
        .iter()
        .position(|row| row.7 == command.environment.as_str() && row.6 == command.token);
    let Some(canonical_index) = identity_index.or(destination_index) else {
        let installation = insert_installation(connection, command).await?;
        return Ok(UpsertPushInstallationOutcome {
            installation,
            created: true,
        });
    };

    if let (Some(identity_index), Some(destination_index)) = (identity_index, destination_index)
        && identity_index != destination_index
    {
        delete_installation_state(connection, rows[destination_index].0).await?;
    }

    let current = rows
        .get(canonical_index)
        .cloned()
        .ok_or(PushRepositoryError::InvalidData)?;
    let rebound = current.1 != command.user_id
        || current.3 != command.installation_id
        || current.4 != command.platform.as_str()
        || current.5 != command.provider.as_str()
        || current.6 != command.token
        || current.7 != command.environment.as_str();
    let owner_epoch = next_owner_epoch(current.2, rebound)?;
    let installation = sqlx::query_as::<_, InstallationRow>(
        "UPDATE push_installations \
         SET user_id = $2, owner_epoch = $3, installation_id = $4, platform = $5, \
             provider = $6, token = $7, environment = $8, \
             message_preview_enabled = $9, last_seen_at = clock_timestamp(), \
             disabled_at = NULL \
         WHERE id = $1 \
         RETURNING id, user_id, owner_epoch, installation_id, platform, provider, token, \
                   environment, message_preview_enabled, last_seen_at, disabled_at",
    )
    .bind(current.0)
    .bind(command.user_id)
    .bind(owner_epoch)
    .bind(&command.installation_id)
    .bind(command.platform.as_str())
    .bind(command.provider.as_str())
    .bind(&command.token)
    .bind(command.environment.as_str())
    .bind(command.message_preview_enabled)
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| database_error("push_installation_upsert", error))?;
    if rebound {
        terminalize_prior_occurrences(connection, current.0, owner_epoch).await?;
    }
    Ok(UpsertPushInstallationOutcome {
        installation: installation_from_row(installation)?,
        created: false,
    })
}

pub(super) async fn update_installation(
    connection: &mut PgConnection,
    command: &UpdatePushInstallationCommand,
) -> Result<PushInstallationRecord, PushRepositoryError> {
    lock_identity(connection, &command.installation_id).await?;
    lock_destination(connection, &command.token).await?;

    let rows = sqlx::query_as::<_, InstallationRow>(
        "WITH target AS ( \
             SELECT environment FROM push_installations WHERE installation_id = $1 \
         ) \
         SELECT id, user_id, owner_epoch, installation_id, platform, provider, token, \
                environment, message_preview_enabled, last_seen_at, disabled_at \
         FROM push_installations \
         WHERE installation_id = $1 \
            OR (environment = (SELECT environment FROM target) AND token = $2) \
         ORDER BY id \
         FOR UPDATE",
    )
    .bind(&command.installation_id)
    .bind(&command.token)
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| database_error("push_installation_update_lock", error))?;
    let current_index = rows
        .iter()
        .position(|row| row.3 == command.installation_id)
        .ok_or(PushRepositoryError::InstallationNotFound)?;
    let current = rows
        .get(current_index)
        .cloned()
        .ok_or(PushRepositoryError::InvalidData)?;
    if current.1 != command.user_id {
        return Err(PushRepositoryError::InstallationNotFound);
    }
    if let Some(conflict) = rows
        .iter()
        .find(|row| row.0 != current.0 && row.7 == current.7 && row.6 == command.token)
    {
        delete_installation_state(connection, conflict.0).await?;
    }

    let token_changed = current.6 != command.token;
    let owner_epoch = next_owner_epoch(current.2, token_changed)?;
    let message_preview_enabled = command.message_preview_enabled.unwrap_or(current.8);
    let installation = sqlx::query_as::<_, InstallationRow>(
        "UPDATE push_installations \
         SET owner_epoch = $2, token = $3, message_preview_enabled = $4, \
             last_seen_at = clock_timestamp(), disabled_at = NULL \
         WHERE id = $1 \
         RETURNING id, user_id, owner_epoch, installation_id, platform, provider, token, \
                   environment, message_preview_enabled, last_seen_at, disabled_at",
    )
    .bind(current.0)
    .bind(owner_epoch)
    .bind(&command.token)
    .bind(message_preview_enabled)
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| database_error("push_installation_update", error))?;
    if token_changed {
        terminalize_prior_occurrences(connection, current.0, owner_epoch).await?;
    }
    installation_from_row(installation)
}

pub(super) async fn delete_installation(
    connection: &mut PgConnection,
    command: &DeletePushInstallationCommand,
) -> Result<(), PushRepositoryError> {
    lock_identity(connection, &command.installation_id).await?;
    let installation_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM push_installations \
         WHERE installation_id = $1 AND user_id = $2 \
         FOR UPDATE",
    )
    .bind(&command.installation_id)
    .bind(command.user_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| database_error("push_installation_delete_lock", error))?
    .ok_or(PushRepositoryError::InstallationNotFound)?;
    delete_installation_state(connection, installation_id).await
}

async fn insert_installation(
    connection: &mut PgConnection,
    command: &UpsertPushInstallationCommand,
) -> Result<PushInstallationRecord, PushRepositoryError> {
    let row = sqlx::query_as::<_, InstallationRow>(
        "INSERT INTO push_installations \
             (id, user_id, installation_id, platform, provider, token, environment, \
              message_preview_enabled) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         RETURNING id, user_id, owner_epoch, installation_id, platform, provider, token, \
                   environment, message_preview_enabled, last_seen_at, disabled_at",
    )
    .bind(command.id)
    .bind(command.user_id)
    .bind(&command.installation_id)
    .bind(command.platform.as_str())
    .bind(command.provider.as_str())
    .bind(&command.token)
    .bind(command.environment.as_str())
    .bind(command.message_preview_enabled)
    .fetch_one(connection)
    .await
    .map_err(|error| database_error("push_installation_insert", error))?;
    installation_from_row(row)
}

async fn lock_identity(
    connection: &mut PgConnection,
    installation_id: &str,
) -> Result<(), PushRepositoryError> {
    advisory_lock(
        connection,
        &format!("push-installation-identity:{installation_id}"),
        "push_installation_identity_lock",
    )
    .await
}

async fn lock_destination(
    connection: &mut PgConnection,
    token: &str,
) -> Result<(), PushRepositoryError> {
    advisory_lock(
        connection,
        &format!("push-installation-destination:{token}"),
        "push_installation_destination_lock",
    )
    .await
}

async fn advisory_lock(
    connection: &mut PgConnection,
    key: &str,
    operation: &'static str,
) -> Result<(), PushRepositoryError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(key)
        .execute(connection)
        .await
        .map_err(|error| database_error(operation, error))?;
    Ok(())
}

async fn terminalize_prior_occurrences(
    connection: &mut PgConnection,
    installation_id: Uuid,
    owner_epoch: i64,
) -> Result<(), PushRepositoryError> {
    sqlx::query(
        "UPDATE push_delivery_intents \
         SET status = 'failed', claim_owner = NULL, lease_expires_at = NULL, \
             next_attempt_at = NULL, last_error_code = 'installation_rebound', \
             failed_at = clock_timestamp() \
         WHERE push_installation_id = $1 AND installation_owner_epoch < $2 \
           AND ( \
               status IN ('pending', 'retryable') \
               OR (status = 'claimed' AND lease_expires_at <= clock_timestamp()) \
           )",
    )
    .bind(installation_id)
    .bind(owner_epoch)
    .execute(connection)
    .await
    .map_err(|error| database_error("push_occurrence_terminalize_rebound", error))?;
    Ok(())
}

async fn delete_installation_state(
    connection: &mut PgConnection,
    installation_id: Uuid,
) -> Result<(), PushRepositoryError> {
    sqlx::query("DELETE FROM push_delivery_intents WHERE push_installation_id = $1")
        .bind(installation_id)
        .execute(&mut *connection)
        .await
        .map_err(|error| database_error("push_occurrence_delete_installation", error))?;
    let deleted = sqlx::query("DELETE FROM push_installations WHERE id = $1")
        .bind(installation_id)
        .execute(connection)
        .await
        .map_err(|error| database_error("push_installation_delete", error))?;
    if deleted.rows_affected() != 1 {
        return Err(PushRepositoryError::InstallationNotFound);
    }
    Ok(())
}

fn next_owner_epoch(current: i64, changed: bool) -> Result<i64, PushRepositoryError> {
    if current <= 0 {
        return Err(PushRepositoryError::InvalidData);
    }
    if changed {
        current
            .checked_add(1)
            .ok_or(PushRepositoryError::InvalidData)
    } else {
        Ok(current)
    }
}

fn installation_from_row(
    row: InstallationRow,
) -> Result<PushInstallationRecord, PushRepositoryError> {
    let owner_epoch = next_owner_epoch(row.2, false)?;
    let platform = PushPlatform::parse(&row.4).ok_or(PushRepositoryError::InvalidData)?;
    if row.5 != PushProviderName::Expo.as_str() {
        return Err(PushRepositoryError::InvalidData);
    }
    let environment = PushEnvironment::parse(&row.7).ok_or(PushRepositoryError::InvalidData)?;
    Ok(PushInstallationRecord {
        id: row.0,
        user_id: row.1,
        owner_epoch,
        installation_id: row.3,
        platform,
        provider: PushProviderName::Expo,
        token: row.6,
        environment,
        message_preview_enabled: row.8,
        last_seen_at: row.9,
        disabled_at: row.10,
    })
}
