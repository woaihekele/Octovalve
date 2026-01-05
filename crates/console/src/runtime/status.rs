use std::sync::Arc;

use tokio::sync::broadcast;
use tokio::sync::RwLock;

use crate::events::ConsoleEvent;
use crate::state::{ConsoleState, TargetStatus};

pub(crate) async fn set_status_and_notify(
    name: &str,
    status: TargetStatus,
    error: Option<String>,
    state: &Arc<RwLock<ConsoleState>>,
    event_tx: &broadcast::Sender<ConsoleEvent>,
) {
    {
        let mut state = state.write().await;
        state.set_status(name, status, error);
    }
    emit_target_update(name, state, event_tx).await;
}

pub(crate) async fn emit_target_update(
    name: &str,
    state: &Arc<RwLock<ConsoleState>>,
    event_tx: &broadcast::Sender<ConsoleEvent>,
) {
    let target = {
        let state = state.read().await;
        state.target_info(name)
    };
    if let Some(target) = target {
        let _ = event_tx.send(ConsoleEvent::TargetUpdated { target });
    }
}
