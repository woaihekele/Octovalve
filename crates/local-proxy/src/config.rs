use anyhow::Context;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub(crate) struct ProxyConfig {
    pub(crate) default_target: Option<String>,
    pub(crate) defaults: Option<ProxyDefaults>,
    pub(crate) targets: Vec<TargetConfig>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProxyDefaults {
    pub(crate) timeout_ms: Option<u64>,
    pub(crate) max_output_bytes: Option<u64>,
    pub(crate) local_bind: Option<String>,
    pub(crate) remote_addr: Option<String>,
    pub(crate) ssh_args: Option<Vec<String>>,
    pub(crate) ssh_password: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TargetConfig {
    pub(crate) name: String,
    pub(crate) desc: String,
    pub(crate) ssh: String,
    pub(crate) remote_addr: Option<String>,
    pub(crate) local_port: u16,
    pub(crate) local_bind: Option<String>,
    pub(crate) ssh_args: Option<Vec<String>>,
    pub(crate) ssh_password: Option<String>,
}

impl Default for ProxyDefaults {
    fn default() -> Self {
        Self {
            timeout_ms: None,
            max_output_bytes: None,
            local_bind: None,
            remote_addr: None,
            ssh_args: None,
            ssh_password: None,
        }
    }
}

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
local_port = 19311
"#;
        let parsed: Result<ProxyConfig, _> = toml::from_str(input);
        assert!(parsed.is_err());
    }
}
