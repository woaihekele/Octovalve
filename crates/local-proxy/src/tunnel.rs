use crate::state::ProxyState;
use std::sync::Arc;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

pub(crate) fn spawn_shutdown_handler(state: Arc<RwLock<ProxyState>>, shutdown: CancellationToken) {
    tokio::spawn(async move {
        let mut sigint = match signal(SignalKind::interrupt()) {
            Ok(signal) => signal,
            Err(err) => {
                tracing::warn!(error = %err, "failed to register SIGINT handler");
                return;
            }
        };
        let mut sigterm = match signal(SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(err) => {
                tracing::warn!(error = %err, "failed to register SIGTERM handler");
                return;
            }
        };
        let mut sighup = match signal(SignalKind::hangup()) {
            Ok(signal) => signal,
            Err(err) => {
                tracing::warn!(error = %err, "failed to register SIGHUP handler");
                return;
            }
        };
        let mut sigquit = match signal(SignalKind::quit()) {
            Ok(signal) => signal,
            Err(err) => {
                tracing::warn!(error = %err, "failed to register SIGQUIT handler");
                return;
            }
        };

        tokio::select! {
            _ = sigint.recv() => {
                tracing::info!("received SIGINT, releasing tunnel forwards");
            }
            _ = sigterm.recv() => {
                tracing::info!("received SIGTERM, releasing tunnel forwards");
            }
            _ = sighup.recv() => {
                tracing::info!("received SIGHUP, releasing tunnel forwards");
            }
            _ = sigquit.recv() => {
                tracing::info!("received SIGQUIT, releasing tunnel forwards");
            }
        }

        shutdown.cancel();
        shutdown_tunnels(state).await;
        std::process::exit(0);
    });
}

pub(crate) async fn shutdown_tunnels(state: Arc<RwLock<ProxyState>>) {
    let tunnel_manager = {
        let state = state.read().await;
        state.tunnel_manager()
    };
    if let Some(manager) = tunnel_manager {
        manager.shutdown().await;
    }
}
