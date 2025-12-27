use crate::cli::Args;
use crate::config::{load_proxy_config, ProxyConfig};
use crate::tunnel_client::TunnelClient;
use serde::Serialize;
use std::collections::HashMap;
use std::time::SystemTime;
use tunnel_protocol::{ForwardPurpose, ForwardSpec};

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
    pub(crate) remote_addr: String,
    pub(crate) local_bind: Option<String>,
    pub(crate) local_port: Option<u16>,
    pub(crate) local_addr: String,
    pub(crate) status: TargetStatus,
    pub(crate) last_seen: Option<SystemTime>,
    pub(crate) last_error: Option<String>,
}

pub(crate) struct ProxyState {
    targets: HashMap<String, TargetRuntime>,
    target_order: Vec<String>,
    default_target: Option<String>,
    tunnel_client: Option<TunnelClient>,
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

impl ProxyState {
    pub(crate) fn target_names(&self) -> Vec<String> {
        self.target_order.clone()
    }

    pub(crate) fn set_tunnel_client(&mut self, client: Option<TunnelClient>) {
        self.tunnel_client = client;
    }

    pub(crate) fn tunnel_client(&self) -> Option<TunnelClient> {
        self.tunnel_client.clone()
    }

    pub(crate) fn default_target(&self) -> Option<String> {
        self.default_target.clone()
    }

    pub(crate) fn target_addr(&self, name: &str) -> anyhow::Result<String> {
        let target = self
            .targets
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown target: {name}"))?;
        Ok(target.local_addr.clone())
    }

    pub(crate) fn list_targets(&mut self) -> Vec<TargetListEntry> {
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

    pub(crate) fn forward_spec(&self, name: &str) -> anyhow::Result<Option<ForwardSpec>> {
        let target = self
            .targets
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown target: {name}"))?;
        if target.ssh.is_none() {
            return Ok(None);
        }
        let bind = target
            .local_bind
            .clone()
            .unwrap_or_else(|| DEFAULT_BIND_HOST.to_string());
        let port = target
            .local_port
            .ok_or_else(|| anyhow::anyhow!("missing local_port for target {name}"))?;
        Ok(Some(ForwardSpec {
            target: target.name.clone(),
            purpose: ForwardPurpose::Data,
            local_bind: bind,
            local_port: port,
            remote_addr: target.remote_addr.clone(),
        }))
    }

    pub(crate) fn forward_specs(&self) -> Vec<ForwardSpec> {
        self.target_order
            .iter()
            .filter_map(|name| self.targets.get(name))
            .filter_map(|target| {
                if target.ssh.is_none() {
                    return None;
                }
                let bind = target
                    .local_bind
                    .clone()
                    .unwrap_or_else(|| DEFAULT_BIND_HOST.to_string());
                let port = target.local_port?;
                Some(ForwardSpec {
                    target: target.name.clone(),
                    purpose: ForwardPurpose::Data,
                    local_bind: bind,
                    local_port: port,
                    remote_addr: target.remote_addr.clone(),
                })
            })
            .collect()
    }

    pub(crate) fn note_tunnel_ready(&mut self, name: &str) {
        if let Some(target) = self.targets.get_mut(name) {
            target.status = TargetStatus::Ready;
            target.last_error = None;
        }
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
    let config = load_proxy_config(&args.config)?;
    build_state_from_config(args, config)
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
        if let Some(ssh) = target.ssh.as_deref() {
            if ssh.trim().is_empty() {
                anyhow::bail!("target {} ssh cannot be empty", target.name);
            }
            if ssh.split_whitespace().count() > 1 {
                anyhow::bail!(
                    "target {} ssh must be a single destination; use ssh_args for options",
                    target.name
                );
            }
        }
        let remote_addr = target.remote_addr.unwrap_or_else(|| default_remote.clone());
        let local_bind = target.local_bind.unwrap_or_else(|| default_bind.clone());
        let local_port = if target.ssh.is_some() {
            Some(
                target
                    .local_port
                    .ok_or_else(|| anyhow::anyhow!("target {} missing local_port", target.name))?,
            )
        } else {
            None
        };
        let local_addr = if let Some(port) = local_port {
            format!("{local_bind}:{port}")
        } else {
            remote_addr.clone()
        };

        let mut runtime = TargetRuntime {
            name: target.name.clone(),
            desc: target.desc,
            ssh: target.ssh,
            remote_addr,
            local_bind: local_port.map(|_| local_bind),
            local_port,
            local_addr,
            status: TargetStatus::Down,
            last_seen: None,
            last_error: None,
        };

        if runtime.ssh.is_none() {
            runtime.status = TargetStatus::Ready;
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
        tunnel_client: None,
    };

    let defaults = ProxyRuntimeDefaults {
        timeout_ms,
        max_output_bytes,
    };
    Ok((state, defaults))
}
