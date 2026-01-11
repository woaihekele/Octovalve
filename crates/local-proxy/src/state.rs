use crate::cli::Args;
use crate::config::{load_proxy_config, ProxyConfig};
use serde::Serialize;
use std::collections::HashMap;
use std::time::SystemTime;

pub(crate) struct ProxyRuntimeDefaults {
    pub(crate) timeout_ms: u64,
    pub(crate) max_output_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
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
    pub(crate) status: TargetStatus,
    pub(crate) last_seen: Option<SystemTime>,
    pub(crate) last_error: Option<String>,
}

pub(crate) struct ProxyState {
    targets: HashMap<String, TargetRuntime>,
    target_order: Vec<String>,
    default_target: Option<String>,
    command_addr: String,
}

#[derive(Serialize)]
pub(crate) struct TargetListEntry {
    pub(crate) name: String,
    pub(crate) desc: String,
    pub(crate) last_seen: Option<String>,
    pub(crate) ssh: Option<String>,
    pub(crate) status: TargetStatus,
    pub(crate) last_error: Option<String>,
}

impl ProxyState {
    pub(crate) fn target_names(&self) -> Vec<String> {
        self.target_order.clone()
    }

    pub(crate) fn default_target(&self) -> Option<String> {
        self.default_target.clone()
    }

    pub(crate) fn target_addr(&self, name: &str) -> anyhow::Result<String> {
        if !self.targets.contains_key(name) {
            return Err(anyhow::anyhow!("unknown target: {name}"));
        }
        Ok(self.command_addr.clone())
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
                status: target.status,
                last_error: target.last_error.clone(),
            })
            .collect()
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
    let command_addr = args.command_addr.clone();

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
        let mut ssh_args = defaults.ssh_args.clone().unwrap_or_default();
        if let Some(extra) = target.ssh_args {
            ssh_args.extend(extra);
        }
        let ssh_password = target
            .ssh_password
            .or_else(|| defaults.ssh_password.clone());
        if ssh_password.is_some() {
            tracing::warn!(
                target = %target.name,
                "ssh_password is set; prefer SSH key auth (keyboard-interactive/2FA is not supported)"
            );
        }

        let status = TargetStatus::Ready;
        let runtime = TargetRuntime {
            name: target.name.clone(),
            desc: target.desc,
            ssh: target.ssh,
            ssh_args,
            ssh_password,
            status,
            last_seen: None,
            last_error: None,
        };

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
        command_addr,
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
    use protocol::config::TargetConfig;
    use std::path::PathBuf;

    fn base_args() -> Args {
        Args {
            config: PathBuf::from("config.toml"),
            client_id: "proxy".to_string(),
            command_addr: "127.0.0.1:19310".to_string(),
            timeout_ms: 30_000,
            max_output_bytes: 1024 * 1024,
        }
    }

    #[test]
    fn resolves_target_addr_to_command_addr() {
        let args = base_args();
        let config = ProxyConfig {
            default_target: None,
            defaults: None,
            targets: vec![TargetConfig {
                name: "dev".to_string(),
                desc: "dev".to_string(),
                hostname: None,
                ip: None,
                ssh: Some("devops@127.0.0.1".to_string()),
                ssh_args: None,
                ssh_password: None,
                terminal_locale: None,
                tty: false,
            }],
        };
        let (mut state, _) = build_state_from_config(&args, config).expect("state");
        assert_eq!(state.target_addr("dev").expect("addr"), "127.0.0.1:19310");
        let targets = state.list_targets();
        assert_eq!(targets[0].status, TargetStatus::Ready);
    }
}
