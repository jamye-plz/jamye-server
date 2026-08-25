use std::error::Error;

use jamye_server::{
    config::AppConfig,
    platform::{logging::init_json_logging, shutdown::wait_for_shutdown_signal},
    transport::realtime::composition,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = AppConfig::from_env()?;
    init_json_logging()?;

    tracing::info!(
        target: "jamye_server",
        environment = ?config.environment(),
        "starting worker process"
    );
    let runtime = composition::worker(&config)?;
    runtime.run_until(wait_for_shutdown_signal()).await;
    tracing::info!(target: "jamye_server", "worker process stopped");
    Ok(())
}
