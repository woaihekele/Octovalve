use crate::config::{
    default_control_remote_addr, default_remote_addr, ConsoleConfig, ConsoleDefaults, TargetConfig,
};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;

use super::{ConsoleState, TargetSpec};

use protocol::config::{
    control_local_addr, control_local_bind, control_local_port, derive_control_addr,
};

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

    Ok(ConsoleState::new(targets, order, config.default_target))
}

fn resolve_target(defaults: &ConsoleDefaults, target: TargetConfig) -> anyhow::Result<TargetSpec> {
    let default_remote = defaults
        .remote_addr
        .clone()
        .unwrap_or_else(default_remote_addr);
    let remote_addr = target.remote_addr.clone().unwrap_or(default_remote);

    let control_remote_addr = target
        .control_remote_addr
        .clone()
        .or_else(|| defaults.control_remote_addr.clone())
        .or_else(|| derive_control_addr(&remote_addr).ok())
        .unwrap_or_else(default_control_remote_addr);

    let control_bind = control_local_bind(Some(defaults), &target);
    let control_local_port = control_local_port(Some(defaults), &target);

    if target.ssh.is_some() && control_local_port.is_none() {
        anyhow::bail!(
            "target {} requires control_local_port (or local_port + offset)",
            target.name
        );
    }

    let control_local_addr = control_local_addr(Some(defaults), &target, control_local_port);

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
        let target = state.target_spec("dev").expect("target");
        assert_eq!(
            target.control_local_addr.as_deref(),
            Some("127.0.0.1:19411")
        );
    }
}
