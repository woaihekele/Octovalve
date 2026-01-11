use crate::config::{ConsoleConfig, ConsoleDefaults, TargetConfig};
use std::collections::{HashMap, HashSet};

use super::{ConsoleState, TargetSpec};

use protocol::config::{parse_ssh_destination, resolve_terminal_locale};

pub(crate) fn build_console_state(config: ConsoleConfig) -> anyhow::Result<ConsoleState> {
    let defaults = config.defaults.unwrap_or_default();
    let mut targets = HashMap::new();
    let mut order = Vec::new();
    let mut seen = HashSet::new();
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
            if parse_ssh_destination(ssh).is_none() {
                anyhow::bail!("target {} ssh must be in the form user@host", resolved.name);
            }
        } else {
            anyhow::bail!("target {} missing ssh destination", resolved.name);
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
    let mut ssh_args = defaults.ssh_args.clone().unwrap_or_default();
    if let Some(extra) = target.ssh_args.clone() {
        ssh_args.extend(extra);
    }
    let ssh_password = target
        .ssh_password
        .clone()
        .or_else(|| defaults.ssh_password.clone());
    let terminal_locale = resolve_terminal_locale(Some(defaults), &target);
    if ssh_password.is_some() {
        tracing::warn!(
            target = %target.name,
            "ssh_password is set; prefer SSH key auth (keyboard-interactive/2FA is not supported)"
        );
    }

    Ok(TargetSpec {
        name: target.name,
        desc: target.desc,
        ssh: target.ssh,
        ssh_args,
        ssh_password,
        terminal_locale,
        tty: target.tty,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_ssh_args_from_defaults_and_target() {
        let config = ConsoleConfig {
            default_target: None,
            defaults: Some(ConsoleDefaults {
                ssh_args: Some(vec![
                    "-o".to_string(),
                    "StrictHostKeyChecking=no".to_string(),
                ]),
                ..Default::default()
            }),
            targets: vec![TargetConfig {
                name: "dev".to_string(),
                desc: "dev".to_string(),
                ssh: Some("devops@127.0.0.1".to_string()),
                ssh_args: Some(vec!["-p".to_string(), "2222".to_string()]),
                ssh_password: None,
                terminal_locale: None,
                tty: false,
            }],
        };
        let state = build_console_state(config).expect("state");
        let target = state.target_spec("dev").expect("target");
        assert_eq!(
            target.ssh_args,
            vec![
                "-o".to_string(),
                "StrictHostKeyChecking=no".to_string(),
                "-p".to_string(),
                "2222".to_string()
            ]
        );
    }

    #[test]
    fn requires_user_in_ssh_destination() {
        let config = ConsoleConfig {
            default_target: None,
            defaults: None,
            targets: vec![TargetConfig {
                name: "dev".to_string(),
                desc: "dev".to_string(),
                ssh: Some("127.0.0.1".to_string()),
                ssh_args: None,
                ssh_password: None,
                terminal_locale: None,
                tty: false,
            }],
        };
        let err = build_console_state(config)
            .err()
            .expect("expected error")
            .to_string();
        assert!(err.contains("user@host"));
    }
}
