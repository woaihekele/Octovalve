use crate::config::{
    default_control_remote_addr, default_remote_addr, ConsoleConfig, ConsoleDefaults, TargetConfig,
};
use crate::control::{ServiceEvent, ServiceSnapshot};
use anyhow::Context;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::time::SystemTime;
use tokio::sync::mpsc;

const DEFAULT_BIND_HOST: &str = "127.0.0.1";
const DEFAULT_CONTROL_PORT_OFFSET: u16 = 100;
const HISTORY_LIMIT: usize = 50;

pub(crate) enum ControlCommand {
    Approve(String),
    Deny(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TargetStatus {
    Ready,
    Down,
}

#[derive(Clone, Debug)]
pub(crate) struct TargetSpec {
    pub(crate) name: String,
    pub(crate) desc: String,
    pub(crate) hostname: Option<String>,
    pub(crate) ip: Option<String>,
    pub(crate) ssh: Option<String>,
    pub(crate) ssh_args: Vec<String>,
    pub(crate) ssh_password: Option<String>,
    pub(crate) terminal_locale: Option<String>,
    pub(crate) control_remote_addr: String,
    pub(crate) control_local_bind: Option<String>,
    pub(crate) control_local_port: Option<u16>,
    pub(crate) control_local_addr: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct TargetInfo {
    pub(crate) name: String,
    pub(crate) hostname: Option<String>,
    pub(crate) ip: Option<String>,
    pub(crate) desc: String,
    pub(crate) status: TargetStatus,
    pub(crate) pending_count: usize,
    pub(crate) last_seen: Option<String>,
    pub(crate) last_error: Option<String>,
    pub(crate) control_addr: String,
    pub(crate) local_addr: Option<String>,
    pub(crate) terminal_available: bool,
    pub(crate) is_default: bool,
}

pub(crate) struct ConsoleState {
    targets: HashMap<String, TargetSpec>,
    order: Vec<String>,
    default_target: Option<String>,
    status: HashMap<String, TargetStatus>,
    pending_count: HashMap<String, usize>,
    last_seen: HashMap<String, SystemTime>,
    last_error: HashMap<String, String>,
    snapshots: HashMap<String, ServiceSnapshot>,
    command_txs: HashMap<String, mpsc::Sender<ControlCommand>>,
}

impl ConsoleState {
    pub(crate) fn update_target_ip(&mut self, name: &str, ip: String) -> Option<TargetInfo> {
        if ip.trim().is_empty() {
            return None;
        }
        let should_update = self
            .targets
            .get(name)
            .map(|target| target.ip.is_none())
            .unwrap_or(false);
        if !should_update {
            return None;
        }
        if let Some(target) = self.targets.get_mut(name) {
            target.ip = Some(ip);
        }
        self.target_info(name)
    }

    pub(crate) fn list_targets(&self) -> Vec<TargetInfo> {
        self.order
            .iter()
            .filter_map(|name| self.targets.get(name))
            .filter_map(|target| self.target_info(&target.name))
            .collect()
    }

    pub(crate) fn target_specs(&self) -> Vec<TargetSpec> {
        self.order
            .iter()
            .filter_map(|name| self.targets.get(name).cloned())
            .collect()
    }

    pub(crate) fn snapshot(&self, name: &str) -> Option<ServiceSnapshot> {
        self.snapshots.get(name).cloned()
    }

    pub(crate) fn target_spec(&self, name: &str) -> Option<TargetSpec> {
        self.targets.get(name).cloned()
    }

    pub(crate) fn target_info(&self, name: &str) -> Option<TargetInfo> {
        let target = self.targets.get(name)?;
        Some(TargetInfo {
            name: target.name.clone(),
            hostname: target.hostname.clone(),
            ip: target.ip.clone(),
            desc: target.desc.clone(),
            status: *self.status.get(&target.name).unwrap_or(&TargetStatus::Down),
            pending_count: *self.pending_count.get(&target.name).unwrap_or(&0),
            last_seen: self.last_seen.get(&target.name).map(format_time),
            last_error: self.last_error.get(&target.name).cloned(),
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
        self.command_txs.insert(name, sender);
    }

    pub(crate) fn command_sender(&self, name: &str) -> Option<mpsc::Sender<ControlCommand>> {
        self.command_txs.get(name).cloned()
    }

    pub(crate) fn set_status(&mut self, name: &str, status: TargetStatus, error: Option<String>) {
        self.status.insert(name.to_string(), status);
        if let Some(err) = error {
            self.last_error.insert(name.to_string(), err);
        } else {
            self.last_error.remove(name);
        }
    }

    pub(crate) fn note_seen(&mut self, name: &str) {
        self.last_seen.insert(name.to_string(), SystemTime::now());
    }

    pub(crate) fn apply_snapshot(&mut self, name: &str, snapshot: ServiceSnapshot) {
        self.pending_count
            .insert(name.to_string(), snapshot.queue.len());
        self.snapshots.insert(name.to_string(), snapshot);
        self.note_seen(name);
    }

    pub(crate) fn apply_event(&mut self, name: &str, event: ServiceEvent) {
        match event {
            ServiceEvent::QueueUpdated(queue) => {
                let entry =
                    self.snapshots
                        .entry(name.to_string())
                        .or_insert_with(|| ServiceSnapshot {
                            queue: Vec::new(),
                            running: Vec::new(),
                            history: Vec::new(),
                            last_result: None,
                        });
                entry.queue = queue;
                self.pending_count
                    .insert(name.to_string(), entry.queue.len());
            }
            ServiceEvent::RunningUpdated(running) => {
                let entry =
                    self.snapshots
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
                    self.snapshots
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

pub(crate) fn build_console_state(config: ConsoleConfig) -> anyhow::Result<ConsoleState> {
    let defaults = config.defaults.unwrap_or_default();
    let mut targets = HashMap::new();
    let mut order = Vec::new();
    let mut seen = HashSet::new();
    let mut local_addr_used = HashSet::new();

    for target in config.targets {
        if target.name.trim().is_empty() {
            anyhow::bail!("target name cannot be empty");
        }
        if seen.contains(&target.name) {
            anyhow::bail!("duplicate target name: {}", target.name);
        }
        seen.insert(target.name.clone());

        let resolved = resolve_target(&defaults, target)?;
        if let Some(ssh) = resolved.ssh.as_ref() {
            if ssh.split_whitespace().count() > 1 {
                anyhow::bail!(
                    "target {} ssh must be a single destination; use ssh_args for options",
                    resolved.name
                );
            }
        }
        if let Some(local_addr) = &resolved.control_local_addr {
            if local_addr_used.contains(local_addr) {
                anyhow::bail!("duplicate control local addr: {}", local_addr);
            }
            local_addr_used.insert(local_addr.clone());
        }

        order.push(resolved.name.clone());
        targets.insert(resolved.name.clone(), resolved);
    }

    if let Some(default_target) = config.default_target.as_ref() {
        if !targets.contains_key(default_target) {
            anyhow::bail!("default_target {} not found in targets", default_target);
        }
    }

    let status = targets
        .keys()
        .map(|name| (name.clone(), TargetStatus::Down))
        .collect();
    let pending_count = targets.keys().map(|name| (name.clone(), 0)).collect();

    Ok(ConsoleState {
        targets,
        order,
        default_target: config.default_target,
        status,
        pending_count,
        last_seen: HashMap::new(),
        last_error: HashMap::new(),
        snapshots: HashMap::new(),
        command_txs: HashMap::new(),
    })
}

fn resolve_target(defaults: &ConsoleDefaults, target: TargetConfig) -> anyhow::Result<TargetSpec> {
    let default_remote = defaults
        .remote_addr
        .clone()
        .unwrap_or_else(default_remote_addr);
    let remote_addr = target.remote_addr.unwrap_or(default_remote);

    let control_remote_addr = target
        .control_remote_addr
        .or_else(|| defaults.control_remote_addr.clone())
        .or_else(|| derive_control_addr(&remote_addr).ok())
        .unwrap_or_else(default_control_remote_addr);

    let control_bind = target
        .control_local_bind
        .or_else(|| target.local_bind.clone())
        .or_else(|| defaults.control_local_bind.clone())
        .or_else(|| defaults.local_bind.clone())
        .unwrap_or_else(|| DEFAULT_BIND_HOST.to_string());

    let offset = defaults
        .control_local_port_offset
        .unwrap_or(DEFAULT_CONTROL_PORT_OFFSET);
    let control_local_port = target
        .control_local_port
        .or_else(|| target.local_port.and_then(|port| port.checked_add(offset)));

    if target.ssh.is_some() && control_local_port.is_none() {
        anyhow::bail!(
            "target {} requires control_local_port (or local_port + offset)",
            target.name
        );
    }

    let control_local_addr = control_local_port.map(|port| format!("{control_bind}:{port}"));

    let mut ssh_args = defaults.ssh_args.clone().unwrap_or_default();
    if let Some(extra) = target.ssh_args {
        ssh_args.extend(extra);
    }
    let ssh_password = target
        .ssh_password
        .or_else(|| defaults.ssh_password.clone());
    let terminal_locale = target
        .terminal_locale
        .or_else(|| defaults.terminal_locale.clone())
        .and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
    if ssh_password.is_some() {
        tracing::warn!(
            target = %target.name,
            "ssh_password is set; prefer SSH key auth (keyboard-interactive/2FA is not supported)"
        );
    }

    let ssh_host = target
        .ssh
        .as_deref()
        .and_then(parse_ssh_host)
        .map(|host| host.to_string());
    let hostname = target.hostname.or_else(|| ssh_host.clone());
    let ip = target.ip.or_else(|| {
        ssh_host
            .as_deref()
            .and_then(|host| host.parse::<IpAddr>().ok().map(|_| host.to_string()))
    });

    Ok(TargetSpec {
        name: target.name,
        desc: target.desc,
        hostname,
        ip,
        ssh: target.ssh,
        ssh_args,
        ssh_password,
        terminal_locale,
        control_remote_addr,
        control_local_bind: control_local_port.map(|_| control_bind),
        control_local_port,
        control_local_addr,
    })
}

fn derive_control_addr(remote_addr: &str) -> anyhow::Result<String> {
    let (host, port) = parse_host_port(remote_addr)?;
    let control_port = port.saturating_add(1);
    Ok(format!("{host}:{control_port}"))
}

pub(crate) fn parse_ssh_host(value: &str) -> Option<&str> {
    if value.is_empty() {
        return None;
    }
    Some(
        value
            .rsplit_once('@')
            .map(|(_, host)| host)
            .unwrap_or(value),
    )
}

fn parse_host_port(addr: &str) -> anyhow::Result<(String, u16)> {
    let (host, port) = addr
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("invalid address {addr}, expected host:port"))?;
    let port = port
        .parse::<u16>()
        .with_context(|| format!("invalid port in address {addr}"))?;
    Ok((host.to_string(), port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_control_addr_from_remote() {
        let addr = derive_control_addr("127.0.0.1:19307").expect("addr");
        assert_eq!(addr, "127.0.0.1:19308");
    }

    #[test]
    fn uses_offset_for_control_local_port() {
        let config = ConsoleConfig {
            default_target: None,
            defaults: None,
            targets: vec![TargetConfig {
                name: "dev".to_string(),
                desc: "dev".to_string(),
                hostname: None,
                ip: None,
                ssh: Some("devops@127.0.0.1".to_string()),
                remote_addr: Some("127.0.0.1:19307".to_string()),
                local_port: Some(19311),
                local_bind: None,
                ssh_args: None,
                ssh_password: None,
                terminal_locale: None,
                control_remote_addr: None,
                control_local_port: None,
                control_local_bind: None,
            }],
        };
        let state = build_console_state(config).expect("state");
        let target = state.targets.get("dev").expect("target");
        assert_eq!(
            target.control_local_addr.as_deref(),
            Some("127.0.0.1:19411")
        );
    }
}
