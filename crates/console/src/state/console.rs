use std::collections::HashMap;
use std::time::SystemTime;

use tokio::sync::mpsc;

use crate::control::{ServiceEvent, ServiceSnapshot};

use super::model::{ControlCommand, TargetInfo, TargetSpec, TargetStatus};

const HISTORY_LIMIT: usize = 50;

struct TargetCache {
    targets: HashMap<String, TargetSpec>,
    order: Vec<String>,
    default_target: Option<String>,
}

struct ConnectionState {
    status: HashMap<String, TargetStatus>,
    last_seen: HashMap<String, SystemTime>,
    last_error: HashMap<String, String>,
    command_txs: HashMap<String, mpsc::Sender<ControlCommand>>,
}

struct SessionState {
    pending_count: HashMap<String, usize>,
    snapshots: HashMap<String, ServiceSnapshot>,
}

pub(crate) struct ConsoleState {
    cache: TargetCache,
    connection: ConnectionState,
    session: SessionState,
}

impl ConsoleState {
    pub(super) fn new(
        targets: HashMap<String, TargetSpec>,
        order: Vec<String>,
        default_target: Option<String>,
    ) -> Self {
        let status = targets
            .keys()
            .map(|name| (name.clone(), TargetStatus::Down))
            .collect();
        let pending_count = targets.keys().map(|name| (name.clone(), 0)).collect();
        Self {
            cache: TargetCache {
                targets,
                order,
                default_target,
            },
            connection: ConnectionState {
                status,
                last_seen: HashMap::new(),
                last_error: HashMap::new(),
                command_txs: HashMap::new(),
            },
            session: SessionState {
                pending_count,
                snapshots: HashMap::new(),
            },
        }
    }

    pub(crate) fn update_target_ip(&mut self, name: &str, ip: String) -> Option<TargetInfo> {
        if ip.trim().is_empty() {
            return None;
        }
        let should_update = self
            .cache
            .targets
            .get(name)
            .map(|target| target.ip.is_none())
            .unwrap_or(false);
        if !should_update {
            return None;
        }
        if let Some(target) = self.cache.targets.get_mut(name) {
            target.ip = Some(ip);
        }
        self.target_info(name)
    }

    pub(crate) fn list_targets(&self) -> Vec<TargetInfo> {
        self.cache
            .order
            .iter()
            .filter_map(|name| self.cache.targets.get(name))
            .filter_map(|target| self.target_info(&target.name))
            .collect()
    }

    pub(crate) fn target_specs(&self) -> Vec<TargetSpec> {
        self.cache
            .order
            .iter()
            .filter_map(|name| self.cache.targets.get(name).cloned())
            .collect()
    }

    pub(crate) fn snapshot(&self, name: &str) -> Option<ServiceSnapshot> {
        self.session.snapshots.get(name).cloned()
    }

    pub(crate) fn target_spec(&self, name: &str) -> Option<TargetSpec> {
        self.cache.targets.get(name).cloned()
    }

    pub(crate) fn target_info(&self, name: &str) -> Option<TargetInfo> {
        let target = self.cache.targets.get(name)?;
        Some(TargetInfo {
            name: target.name.clone(),
            hostname: target.hostname.clone(),
            ip: target.ip.clone(),
            desc: target.desc.clone(),
            status: *self
                .connection
                .status
                .get(&target.name)
                .unwrap_or(&TargetStatus::Down),
            pending_count: *self
                .session
                .pending_count
                .get(&target.name)
                .unwrap_or(&0),
            last_seen: self.connection.last_seen.get(&target.name).map(format_time),
            last_error: self.connection.last_error.get(&target.name).cloned(),
            control_addr: target
                .control_local_addr
                .clone()
                .unwrap_or_else(|| target.control_remote_addr.clone()),
            local_addr: target.control_local_addr.clone(),
            terminal_available: target
                .ssh
                .as_deref()
                .map(|ssh| !ssh.trim().is_empty())
                .unwrap_or(false),
            is_default: self
                .cache
                .default_target
                .as_ref()
                .map(|default| default == &target.name)
                .unwrap_or(false),
        })
    }

    pub(crate) fn register_command_sender(
        &mut self,
        name: String,
        sender: mpsc::Sender<ControlCommand>,
    ) {
        self.connection.command_txs.insert(name, sender);
    }

    pub(crate) fn command_sender(&self, name: &str) -> Option<mpsc::Sender<ControlCommand>> {
        self.connection.command_txs.get(name).cloned()
    }

    pub(crate) fn set_status(&mut self, name: &str, status: TargetStatus, error: Option<String>) {
        self.connection.status.insert(name.to_string(), status);
        if let Some(err) = error {
            self.connection.last_error.insert(name.to_string(), err);
        } else {
            self.connection.last_error.remove(name);
        }
    }

    pub(crate) fn note_seen(&mut self, name: &str) {
        self.connection
            .last_seen
            .insert(name.to_string(), SystemTime::now());
    }

    pub(crate) fn apply_snapshot(&mut self, name: &str, snapshot: ServiceSnapshot) {
        self.session
            .pending_count
            .insert(name.to_string(), snapshot.queue.len());
        self.session.snapshots.insert(name.to_string(), snapshot);
        self.note_seen(name);
    }

    pub(crate) fn apply_event(&mut self, name: &str, event: ServiceEvent) {
        match event {
            ServiceEvent::QueueUpdated(queue) => {
                let entry =
                    self.session
                        .snapshots
                        .entry(name.to_string())
                        .or_insert_with(|| ServiceSnapshot {
                            queue: Vec::new(),
                            running: Vec::new(),
                            history: Vec::new(),
                            last_result: None,
                        });
                entry.queue = queue;
                self.session
                    .pending_count
                    .insert(name.to_string(), entry.queue.len());
            }
            ServiceEvent::RunningUpdated(running) => {
                let entry =
                    self.session
                        .snapshots
                        .entry(name.to_string())
                        .or_insert_with(|| ServiceSnapshot {
                            queue: Vec::new(),
                            running: Vec::new(),
                            history: Vec::new(),
                            last_result: None,
                        });
                entry.running = running;
            }
            ServiceEvent::ResultUpdated(result) => {
                let entry =
                    self.session
                        .snapshots
                        .entry(name.to_string())
                        .or_insert_with(|| ServiceSnapshot {
                            queue: Vec::new(),
                            running: Vec::new(),
                            history: Vec::new(),
                            last_result: None,
                        });
                entry.last_result = Some(result.clone());
                entry.history.insert(0, result);
                if entry.history.len() > HISTORY_LIMIT {
                    entry.history.truncate(HISTORY_LIMIT);
                }
            }
            ServiceEvent::ConnectionsChanged => {}
        }
        self.note_seen(name);
    }
}

fn format_time(time: &SystemTime) -> String {
    humantime::format_rfc3339(*time).to_string()
}
