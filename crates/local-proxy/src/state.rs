use crate::cli::Args;
use crate::config::{load_proxy_config, ProxyConfig};
use crate::tunnel::spawn_tunnel;
use serde::Serialize;
use std::collections::HashMap;
use std::time::SystemTime;
use tokio::process::Child;

const DEFAULT_REMOTE_ADDR: &str = "127.0.0.1:19307";
const DEFAULT_BIND_HOST: &str = "127.0.0.1";

pub(crate) struct ProxyRuntimeDefaults {
    pub(crate) timeout_ms: u64,
    pub(crate) max_output_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TargetStatus {
    Ready,
    Down,
}

pub(crate) struct TargetRuntime {
    pub(crate) name: String,
    pub(crate) desc: String,
    pub(crate) ssh: Option<String>,
    pub(crate) ssh_args: Vec<String>,
    pub(crate) ssh_password: Option<String>,
    pub(crate) remote_addr: String,
    pub(crate) local_bind: Option<String>,
    pub(crate) local_port: Option<u16>,
    pub(crate) local_addr: String,
    pub(crate) status: TargetStatus,
    pub(crate) last_seen: Option<SystemTime>,
    pub(crate) last_error: Option<String>,
    pub(crate) tunnel: Option<Child>,
    pub(crate) tunnel_pgid: Option<libc::pid_t>,
}

pub(crate) struct ProxyState {
    targets: HashMap<String, TargetRuntime>,
    target_order: Vec<String>,
    default_target: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct TargetListEntry {
    pub(crate) name: String,
    pub(crate) desc: String,
    pub(crate) last_seen: Option<String>,
    pub(crate) ssh: Option<String>,
    pub(crate) remote_addr: String,
    pub(crate) local_addr: String,
}

impl TargetRuntime {
    fn refresh_status(&mut self) {
        if let Some(child) = self.tunnel.as_mut() {
            match child.try_wait() {
                Ok(None) => {
                    self.status = TargetStatus::Ready;
                }
                Ok(Some(status)) => {
                    self.status = TargetStatus::Down;
                    self.tunnel = None;
                    self.tunnel_pgid = None;
                    self.last_error = Some(format!("ssh exited: {status}"));
                }
                Err(err) => {
                    self.status = TargetStatus::Down;
                    self.tunnel_pgid = None;
                    self.last_error = Some(format!("ssh status check failed: {err}"));
                }
            }
        } else if self.ssh.is_some() {
            self.status = TargetStatus::Down;
        } else {
            self.status = TargetStatus::Ready;
        }
    }
}

impl ProxyState {
    pub(crate) fn target_names(&self) -> Vec<String> {
        self.target_order.clone()
    }

    pub(crate) fn default_target(&self) -> Option<String> {
        self.default_target.clone()
    }

    pub(crate) fn get_target_mut(&mut self, name: &str) -> Option<&mut TargetRuntime> {
        self.targets.get_mut(name)
    }

    pub(crate) fn ensure_all_tunnels(&mut self) {
        for name in &self.target_order {
            if let Some(target) = self.targets.get_mut(name) {
                if target.ssh.is_none() {
                    target.status = TargetStatus::Ready;
                    continue;
                }
                target.refresh_status();
                if target.status != TargetStatus::Ready {
                    if let Err(err) = spawn_tunnel(target) {
                        target.status = TargetStatus::Down;
                        target.last_error = Some(err.to_string());
                        tracing::warn!(target = %target.name, error = %err, "failed to establish ssh tunnel");
                    }
                }
            }
        }
    }

    fn refresh_statuses(&mut self) {
        for name in &self.target_order {
            if let Some(target) = self.targets.get_mut(name) {
                target.refresh_status();
            }
        }
    }

    pub(crate) fn list_targets(&mut self) -> Vec<TargetListEntry> {
        self.refresh_statuses();
        self.target_order
            .iter()
            .filter_map(|name| self.targets.get(name))
            .map(|target| TargetListEntry {
                name: target.name.clone(),
                desc: target.desc.clone(),
                last_seen: target.last_seen.map(format_time),
                ssh: target.ssh.clone(),
                remote_addr: target.remote_addr.clone(),
                local_addr: target.local_addr.clone(),
            })
            .collect()
    }

    pub(crate) fn ensure_tunnel(&mut self, name: &str) -> anyhow::Result<String> {
        let target = self
            .targets
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("unknown target: {name}"))?;
        target.refresh_status();
        if target.ssh.is_some() && target.status != TargetStatus::Ready {
            spawn_tunnel(target)?;
        }
        Ok(target.local_addr.clone())
    }

    pub(crate) fn note_success(&mut self, name: &str) {
        if let Some(target) = self.targets.get_mut(name) {
            target.last_seen = Some(SystemTime::now());
            target.status = TargetStatus::Ready;
            target.last_error = None;
        }
    }

    pub(crate) fn note_failure(&mut self, name: &str, err: &str) {
        if let Some(target) = self.targets.get_mut(name) {
            target.status = TargetStatus::Down;
            target.last_error = Some(err.to_string());
        }
    }
}

fn format_time(time: SystemTime) -> String {
    humantime::format_rfc3339(time).to_string()
}

pub(crate) fn build_proxy_state(args: &Args) -> anyhow::Result<(ProxyState, ProxyRuntimeDefaults)> {
    if let Some(path) = &args.config {
        let config = load_proxy_config(path)?;
        build_state_from_config(args, config)
    } else {
        build_state_from_args(args)
    }
}

fn build_state_from_args(args: &Args) -> anyhow::Result<(ProxyState, ProxyRuntimeDefaults)> {
    let target = TargetRuntime {
        name: "default".to_string(),
        desc: "default remote".to_string(),
        ssh: None,
        ssh_args: Vec::new(),
        ssh_password: None,
        remote_addr: args.remote_addr.clone(),
        local_bind: None,
        local_port: None,
        local_addr: args.remote_addr.clone(),
        status: TargetStatus::Ready,
        last_seen: None,
        last_error: None,
        tunnel: None,
        tunnel_pgid: None,
    };
    let mut targets = HashMap::new();
    targets.insert(target.name.clone(), target);
    let state = ProxyState {
        targets,
        target_order: vec!["default".to_string()],
        default_target: Some("default".to_string()),
    };
    let defaults = ProxyRuntimeDefaults {
        timeout_ms: args.timeout_ms,
        max_output_bytes: args.max_output_bytes,
    };
    Ok((state, defaults))
}

fn build_state_from_config(
    args: &Args,
    config: ProxyConfig,
) -> anyhow::Result<(ProxyState, ProxyRuntimeDefaults)> {
    let defaults = config.defaults.unwrap_or_default();
    let default_remote = defaults
        .remote_addr
        .unwrap_or_else(|| DEFAULT_REMOTE_ADDR.to_string());
    let default_bind = defaults
        .local_bind
        .unwrap_or_else(|| DEFAULT_BIND_HOST.to_string());
    let default_ssh_args = defaults.ssh_args.unwrap_or_default();
    let default_ssh_password = defaults.ssh_password.clone();

    let timeout_ms = defaults.timeout_ms.unwrap_or(args.timeout_ms);
    let max_output_bytes = defaults.max_output_bytes.unwrap_or(args.max_output_bytes);

    let mut targets = HashMap::new();
    let mut order = Vec::new();

    for target in config.targets {
        if target.name.trim().is_empty() {
            anyhow::bail!("target name cannot be empty");
        }
        if targets.contains_key(&target.name) {
            anyhow::bail!("duplicate target name: {}", target.name);
        }
        if target.ssh.trim().is_empty() {
            anyhow::bail!("target {} ssh cannot be empty", target.name);
        }
        if target.ssh.split_whitespace().count() > 1 {
            anyhow::bail!(
                "target {} ssh must be a single destination; use ssh_args for options",
                target.name
            );
        }
        let remote_addr = target
            .remote_addr
            .unwrap_or_else(|| default_remote.clone());
        let local_bind = target
            .local_bind
            .unwrap_or_else(|| default_bind.clone());
        let local_addr = format!("{local_bind}:{}", target.local_port);

        let mut ssh_args = default_ssh_args.clone();
        if let Some(extra) = target.ssh_args {
            ssh_args.extend(extra);
        }
        let ssh_password = target
            .ssh_password
            .or_else(|| default_ssh_password.clone());

        let mut runtime = TargetRuntime {
            name: target.name.clone(),
            desc: target.desc,
            ssh: Some(target.ssh),
            ssh_args,
            ssh_password,
            remote_addr,
            local_bind: Some(local_bind),
            local_port: Some(target.local_port),
            local_addr,
            status: TargetStatus::Down,
            last_seen: None,
            last_error: None,
            tunnel: None,
            tunnel_pgid: None,
        };

        if let Err(err) = spawn_tunnel(&mut runtime) {
            runtime.status = TargetStatus::Down;
            runtime.last_error = Some(err.to_string());
        }

        order.push(runtime.name.clone());
        targets.insert(runtime.name.clone(), runtime);
    }

    if let Some(default_target) = config.default_target.as_ref() {
        if !targets.contains_key(default_target) {
            anyhow::bail!("default_target {} not found in targets", default_target);
        }
    }

    let state = ProxyState {
        targets,
        target_order: order,
        default_target: config.default_target,
    };

    let defaults = ProxyRuntimeDefaults {
        timeout_ms,
        max_output_bytes,
    };
    Ok((state, defaults))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_state_from_args_sets_default_target() {
        let args = Args {
            remote_addr: "127.0.0.1:19306".to_string(),
            config: None,
            client_id: "local-proxy".to_string(),
            timeout_ms: 10,
            max_output_bytes: 20,
        };
        let (state, defaults) = build_state_from_args(&args).expect("state");
        assert!(state.targets.contains_key("default"));
        assert_eq!(defaults.timeout_ms, 10);
        assert_eq!(defaults.max_output_bytes, 20);
    }
}
