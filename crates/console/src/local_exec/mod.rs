mod audit;
mod events;
mod executor;
mod history;
mod output;
mod policy;
mod process;
mod server;
mod service;
mod snapshots;
mod stream;
#[cfg(test)]
mod test_utils;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::broadcast;
use tokio::sync::RwLock;

use crate::events::ConsoleEvent;
use crate::runtime::emit_target_update;
use crate::state::{ConsoleState, ControlCommand, TargetStatus};

pub(crate) use policy::PolicyConfig;
use policy::Whitelist;
use service::TargetServiceHandle;

pub(crate) async fn spawn_local_exec(
    listen_addr: SocketAddr,
    policy: PolicyConfig,
    audit_root: PathBuf,
    state: Arc<RwLock<ConsoleState>>,
    event_tx: broadcast::Sender<ConsoleEvent>,
) -> anyhow::Result<()> {
    let whitelist = Arc::new(Whitelist::from_config(&policy.whitelist)?);
    let limits = Arc::new(policy.limits);
    let audit_root = Arc::new(audit_root);
    std::fs::create_dir_all(&*audit_root)?;

    let targets = {
        let guard = state.read().await;
        guard.target_specs()
    };

    let mut services: HashMap<String, TargetServiceHandle> = HashMap::new();
    for target in targets {
        if target
            .ssh
            .as_deref()
            .map(|value| value.trim().is_empty())
            .unwrap_or(true)
        {
            let message = "ssh not configured".to_string();
            {
                let mut guard = state.write().await;
                guard.set_status(&target.name, TargetStatus::Down, Some(message));
            }
            emit_target_update(&target.name, &state, &event_tx).await;
            continue;
        }
        let output_dir = Arc::new(target_audit_dir(&audit_root, &target.name));
        std::fs::create_dir_all(&*output_dir)?;
        let handle = service::spawn_service(
            target.clone(),
            Arc::clone(&whitelist),
            Arc::clone(&limits),
            Arc::clone(&output_dir),
            Arc::clone(&state),
            event_tx.clone(),
        );
        {
            let mut guard = state.write().await;
            guard.register_command_sender(target.name.clone(), handle.command_tx.clone());
            guard.apply_snapshot(&target.name, handle.snapshot.clone());
            guard.set_status(&target.name, TargetStatus::Ready, None);
        }
        emit_target_update(&target.name, &state, &event_tx).await;
        services.insert(target.name, handle);
    }

    server::spawn_command_server(listen_addr, services, Arc::clone(&whitelist)).await?;
    Ok(())
}

fn target_audit_dir(root: &Path, target: &str) -> PathBuf {
    let sanitized = target.replace(['/', '\\'], "_");
    root.join(sanitized)
}

pub(crate) async fn send_control_command(
    name: &str,
    command: ControlCommand,
    state: &Arc<RwLock<ConsoleState>>,
) -> Result<(), String> {
    let sender = state.read().await.command_sender(name);
    let Some(sender) = sender else {
        return Err("command channel not available".to_string());
    };
    sender
        .send(command)
        .await
        .map_err(|_| format!("command channel unavailable for target {}", name))
}
