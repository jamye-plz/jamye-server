use std::{error::Error, io};

use jamye_server::{
    config::{
        AppConfig, account_deletion::AccountDeletionConfig, object_storage::ObjectStorageConfig,
        push::PushConfig,
    },
    platform::{logging::init_json_logging, shutdown::wait_for_shutdown_signal},
    transport::http::composition,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = AppConfig::from_env()?;
    let push = PushConfig::from_env(config.environment())?;
    let object_storage = ObjectStorageConfig::from_env(config.environment())?.ok_or_else(|| {
        io::Error::other("object storage configuration is required for worker cleanup")
    })?;
    let cleanup = AccountDeletionConfig::from_env()?;
    init_json_logging()?;

    tracing::info!(
        target: "jamye_server",
        environment = ?config.environment(),
        "starting worker process"
    );
    let runtime = composition::worker(&config, &push, &object_storage, &cleanup)?;
    runtime.run_until(wait_for_shutdown_signal()).await?;
    tracing::info!(target: "jamye_server", "worker process stopped");
    Ok(())
}
