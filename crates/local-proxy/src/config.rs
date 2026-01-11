use anyhow::Context;
pub(crate) use protocol::config::ProxyConfig;
use std::path::PathBuf;

pub(crate) fn load_proxy_config(path: &PathBuf) -> anyhow::Result<ProxyConfig> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    let config: ProxyConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse config {}", path.display()))?;
    if config.targets.is_empty() {
        anyhow::bail!("config must include at least one target");
    }
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
}
