//! Operating-system signal handling and bounded HTTP connection draining.

use std::{error::Error, fmt, future::Future, future::IntoFuture, time::Duration};

use axum::Router;
use tokio::{net::TcpListener, sync::oneshot, time::timeout};

pub async fn serve_with_graceful_shutdown<F>(
    listener: TcpListener,
    app: Router,
    shutdown: F,
    grace_period: Duration,
) -> Result<(), ServerError>
where
    F: Future<Output = ()> + Send + 'static,
{
    let (drain_started_tx, drain_started_rx) = oneshot::channel();
    let server = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown.await;
            let _send_result = drain_started_tx.send(());
        })
        .into_future();
    tokio::pin!(server);

    tokio::select! {
        result = &mut server => result.map_err(ServerError::Serve),
        drain_started = drain_started_rx => {
            if drain_started.is_err() {
                return (&mut server).await.map_err(ServerError::Serve);
            }
            match timeout(grace_period, &mut server).await {
                Ok(result) => result.map_err(ServerError::Serve),
                Err(_) => Err(ServerError::DrainTimedOut { grace_period }),
            }
        }
    }
}
#[cfg(unix)]
pub async fn wait_for_shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let interrupt = tokio::signal::ctrl_c();
    match signal(SignalKind::terminate()) {
        Ok(mut terminate) => {
            tokio::select! {
                result = interrupt => log_signal_error(result),
                _ = terminate.recv() => {},
            }
        }
        Err(_) => {
            tracing::error!(
                failure_kind = "signal_registration",
                "failed to register SIGTERM"
            );
            log_signal_error(interrupt.await);
        }
    }
}

#[cfg(not(unix))]
pub async fn wait_for_shutdown_signal() {
    log_signal_error(tokio::signal::ctrl_c().await);
}

fn log_signal_error(result: std::io::Result<()>) {
    if result.is_err() {
        tracing::error!(
            failure_kind = "signal_listener",
            "shutdown signal listener failed"
        );
    }
}

#[derive(Debug)]
pub enum ServerError {
    Serve(std::io::Error),
    DrainTimedOut { grace_period: Duration },
}

impl fmt::Display for ServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serve(_) => formatter.write_str("HTTP server failed"),
            Self::DrainTimedOut { grace_period } => write!(
                formatter,
                "HTTP server did not drain within {} seconds",
                grace_period.as_secs()
            ),
        }
    }
}

impl Error for ServerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Serve(error) => Some(error),
            Self::DrainTimedOut { .. } => None,
        }
    }
}
