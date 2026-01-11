use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProxyConfig {
    pub default_target: Option<String>,
    pub defaults: Option<ProxyDefaults>,
    pub targets: Vec<TargetConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProxyDefaults {
    pub timeout_ms: Option<u64>,
    pub max_output_bytes: Option<u64>,
    pub ssh_args: Option<Vec<String>>,
    pub ssh_password: Option<String>,
    pub terminal_locale: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TargetConfig {
    pub name: String,
    pub desc: String,
    pub hostname: Option<String>,
    pub ip: Option<String>,
    pub ssh: Option<String>,
    pub ssh_args: Option<Vec<String>>,
    pub ssh_password: Option<String>,
    pub terminal_locale: Option<String>,
    #[serde(default)]
    pub tty: bool,
}

impl Default for ProxyDefaults {
    fn default() -> Self {
        Self {
            timeout_ms: None,
            max_output_bytes: None,
            ssh_args: None,
            ssh_password: None,
            terminal_locale: None,
        }
    }
}

pub fn parse_ssh_host(value: &str) -> Option<&str> {
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

pub fn resolve_terminal_locale(
    defaults: Option<&ProxyDefaults>,
    target: &TargetConfig,
) -> Option<String> {
    let target_locale = target
        .terminal_locale
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    let default_locale = defaults
        .and_then(|value| value.terminal_locale.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    target_locale.or(default_locale)
}

pub fn resolve_target_host_info(target: &TargetConfig) -> (Option<String>, Option<String>) {
    let ssh_host = target
        .ssh
        .as_deref()
        .and_then(parse_ssh_host)
        .map(|host| host.to_string());
    let hostname = target.hostname.clone().or_else(|| ssh_host.clone());
    let ip = target.ip.clone().or_else(|| {
        ssh_host
            .as_deref()
            .and_then(|host| host.parse::<IpAddr>().ok().map(|_| host.to_string()))
    });
    (hostname, ip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_terminal_locale_from_target_or_default() {
        let defaults = ProxyDefaults {
            terminal_locale: Some("zh_CN.UTF-8".to_string()),
            ..Default::default()
        };
        let target = TargetConfig {
            name: "dev".to_string(),
            desc: "dev".to_string(),
            hostname: None,
            ip: None,
            ssh: None,
            ssh_args: None,
            ssh_password: None,
            terminal_locale: Some("  ".to_string()),
            tty: false,
        };
        assert_eq!(
            resolve_terminal_locale(Some(&defaults), &target),
            Some("zh_CN.UTF-8".to_string())
        );
    }

    #[test]
    fn resolves_host_info_from_ssh() {
        let target = TargetConfig {
            name: "dev".to_string(),
            desc: "dev".to_string(),
            hostname: None,
            ip: None,
            ssh: Some("user@127.0.0.1".to_string()),
            ssh_args: None,
            ssh_password: None,
            terminal_locale: None,
            tty: false,
        };
        let (hostname, ip) = resolve_target_host_info(&target);
        assert_eq!(hostname.as_deref(), Some("127.0.0.1"));
        assert_eq!(ip.as_deref(), Some("127.0.0.1"));
    }
}
