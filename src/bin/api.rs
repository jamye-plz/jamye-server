use std::{error::Error, io};

use jamye_server::{
    adapters::object_storage::media::{BucketLifecycle, S3BucketBackend},
    config::{
        AppConfig, auth::AuthConfig, object_storage::ObjectStorageConfig,
        rate_limit::RateLimitConfig,
    },
    platform::{
        logging::init_json_logging,
        shutdown::{serve_with_graceful_shutdown, wait_for_shutdown_signal},
    },
    transport::http::composition,
};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = AppConfig::from_env()?;
    let auth = AuthConfig::from_env()?;
    let rate_limits = RateLimitConfig::from_env()?;
    let object_storage = ObjectStorageConfig::from_env(config.environment())?;
    init_json_logging()?;

    tracing::info!(
        target: "jamye_server",
        environment = ?config.environment(),
        listen_address = %config.listen_address(),
        "starting API process"
    );

    // Bucket lifecycle is startup-only, outside every HTTP command transaction.
    let configured_storage = object_storage
        .as_ref()
        .ok_or_else(|| io::Error::other("object storage configuration is required"))?;
    let lifecycle = BucketLifecycle::new(
        S3BucketBackend::new(configured_storage),
        configured_storage.bucket(),
    );
    lifecycle.ensure_bucket().await?;
    let app =
        composition::router_with_runtime(&config, &auth, &rate_limits, object_storage.as_ref())?;
    let listener = TcpListener::bind(config.listen_address()).await?;
    serve_with_graceful_shutdown(
        listener,
        app,
        wait_for_shutdown_signal(),
        config.shutdown_grace(),
    )
    .await?;

    tracing::info!(target: "jamye_server", "API process stopped");
    Ok(())
}
