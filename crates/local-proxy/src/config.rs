use anyhow::Context;
use protocol::config::parse_ssh_destination;
pub(crate) use protocol::config::ProxyConfig;
use std::path::PathBuf;

fn validate_proxy_config(config: &ProxyConfig) -> anyhow::Result<()> {
    if config.targets.is_empty() {
        anyhow::bail!("config must include at least one target");
    }
    for (index, target) in config.targets.iter().enumerate() {
        let label = if target.name.trim().is_empty() {
            format!("target[{}]", index)
        } else {
            format!("target {}", target.name.trim())
        };
        let ssh = target.ssh.as_deref().unwrap_or("").trim();
        if ssh.is_empty() {
            anyhow::bail!("{} must set ssh (user@host)", label);
        }
        if parse_ssh_destination(ssh).is_none() {
            anyhow::bail!("{} ssh must be user@host", label);
        }
    }
    Ok(())
}

pub(crate) fn load_proxy_config(path: &PathBuf) -> anyhow::Result<ProxyConfig> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    let config: ProxyConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse config {}", path.display()))?;
    validate_proxy_config(&config)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_requires_desc() {
        let input = r#"
[[targets]]
name = "dev"
ssh = "devops@127.0.0.1"
"#;
        let parsed: Result<ProxyConfig, _> = toml::from_str(input);
        assert!(parsed.is_err());
    }

    #[test]
    fn config_requires_ssh_user_and_host() {
        let input = r#"
[[targets]]
name = "dev"
desc = "dev"
ssh = "127.0.0.1"
"#;
        let parsed: ProxyConfig = toml::from_str(input).unwrap();
        assert!(validate_proxy_config(&parsed).is_err());
    }

    #[test]
    fn config_requires_ssh_value() {
        let input = r#"
[[targets]]
name = "dev"
desc = "dev"
"#;
        let parsed: ProxyConfig = toml::from_str(input).unwrap();
        assert!(validate_proxy_config(&parsed).is_err());
    }

    #[test]
    fn config_accepts_user_and_host() {
        let input = r#"
[[targets]]
name = "dev"
desc = "dev"
ssh = "devops@127.0.0.1"
"#;
        let parsed: ProxyConfig = toml::from_str(input).unwrap();
        assert!(validate_proxy_config(&parsed).is_ok());
    }
}
