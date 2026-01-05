mod handlers;
mod snapshots;
mod state;

use crate::activity::ActivityTracker;
use crate::layers::policy::config::LimitsConfig;
use crate::layers::policy::whitelist::Whitelist;
use crate::layers::service::events::{ServerEvent, ServiceCommand, ServiceEvent};
use protocol::control::ResultSnapshot;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

use self::handlers::{handle_command, handle_result_snapshot, handle_server_event};
use self::state::ServiceState;

pub(crate) fn run_tui_service(
    listener: tokio::net::TcpListener,
    whitelist: Arc<Whitelist>,
    limits: Arc<LimitsConfig>,
    output_dir: Arc<PathBuf>,
    auto_approve_allowed: bool,
    history: Vec<ResultSnapshot>,
    history_limit: usize,
    event_tx: broadcast::Sender<ServiceEvent>,
    cmd_rx: mpsc::Receiver<ServiceCommand>,
    activity: Arc<ActivityTracker>,
) {
    let (server_tx, server_rx) = mpsc::channel::<ServerEvent>(128);
    let (result_tx, result_rx) = mpsc::channel::<ResultSnapshot>(128);
    crate::layers::service::server::spawn_accept_loop(
        listener,
        server_tx,
        Arc::clone(&output_dir),
        Arc::clone(&whitelist),
        activity,
    );
    tokio::spawn(async move {
        service_loop(
            server_rx,
            cmd_rx,
            result_rx,
            result_tx,
            event_tx,
            whitelist,
            limits,
            output_dir,
            auto_approve_allowed,
            history,
            history_limit,
        )
        .await;
    });
}

async fn service_loop(
    mut server_rx: mpsc::Receiver<ServerEvent>,
    mut cmd_rx: mpsc::Receiver<ServiceCommand>,
    mut result_rx: mpsc::Receiver<ResultSnapshot>,
    result_tx: mpsc::Sender<ResultSnapshot>,
    event_tx: broadcast::Sender<ServiceEvent>,
    whitelist: Arc<Whitelist>,
    limits: Arc<LimitsConfig>,
    output_dir: Arc<PathBuf>,
    auto_approve_allowed: bool,
    history: Vec<ResultSnapshot>,
    history_limit: usize,
) {
    let mut state = ServiceState::new(history, history_limit);
    loop {
        tokio::select! {
            Some(event) = server_rx.recv() => {
                handle_server_event(
                    event,
                    &mut state,
                    &event_tx,
                    &result_tx,
                    &whitelist,
                    &limits,
                    &output_dir,
                    auto_approve_allowed,
                )
                .await;
            }
            Some(command) = cmd_rx.recv() => {
                handle_command(
                    command,
                    &mut state,
                    &event_tx,
                    &result_tx,
                    &whitelist,
                    &limits,
                    &output_dir,
                )
                .await;
            }
            Some(result) = result_rx.recv() => {
                handle_result_snapshot(result, &mut state, &event_tx);
            }
            else => break,
        }
    }
}
