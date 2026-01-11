use serde::{Deserialize, Serialize};

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

pub fn parse_ssh_destination(value: &str) -> Option<(String, String)> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.split_whitespace().count() > 1 {
        return None;
    }
    let (user, host) = trimmed.rsplit_once('@')?;
    let user = user.trim();
    let host = host.trim();
    if user.is_empty() || host.is_empty() {
        return None;
    }
    Some((user.to_string(), host.to_string()))
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
    fn parse_ssh_destination_requires_user_and_host() {
        assert!(parse_ssh_destination("devops@127.0.0.1").is_some());
        assert!(parse_ssh_destination("127.0.0.1").is_none());
        assert!(parse_ssh_destination("devops@").is_none());
        assert!(parse_ssh_destination("@host").is_none());
    }
}
