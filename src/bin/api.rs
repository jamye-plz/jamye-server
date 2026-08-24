use std::error::Error;

use jamye_server::{
    config::AppConfig,
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
    init_json_logging()?;

    tracing::info!(
        target: "jamye_server",
        environment = ?config.environment(),
        listen_address = %config.listen_address(),
        "starting API process"
    );

    let app = composition::router(&config)?;
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
