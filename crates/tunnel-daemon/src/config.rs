use anyhow::Context;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub(crate) struct DaemonConfig {
    pub(crate) defaults: Option<DaemonDefaults>,
    pub(crate) targets: Vec<TargetConfig>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DaemonDefaults {
    pub(crate) ssh_args: Option<Vec<String>>,
    pub(crate) ssh_password: Option<String>,
    pub(crate) local_bind: Option<String>,
    pub(crate) remote_addr: Option<String>,
    pub(crate) control_remote_addr: Option<String>,
    pub(crate) control_local_bind: Option<String>,
    pub(crate) control_local_port_offset: Option<u16>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TargetConfig {
    pub(crate) name: String,
    pub(crate) ssh: Option<String>,
    pub(crate) remote_addr: Option<String>,
    pub(crate) local_port: Option<u16>,
    pub(crate) local_bind: Option<String>,
    pub(crate) ssh_args: Option<Vec<String>>,
    pub(crate) ssh_password: Option<String>,
    pub(crate) control_remote_addr: Option<String>,
    pub(crate) control_local_port: Option<u16>,
    pub(crate) control_local_bind: Option<String>,
}

impl Default for DaemonDefaults {
    fn default() -> Self {
        Self {
            ssh_args: None,
            ssh_password: None,
            local_bind: None,
            remote_addr: None,
            control_remote_addr: None,
            control_local_bind: None,
            control_local_port_offset: None,
        }
    }
}

pub(crate) fn load_daemon_config(path: &PathBuf) -> anyhow::Result<DaemonConfig> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    let config: DaemonConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse config {}", path.display()))?;
    if config.targets.is_empty() {
        anyhow::bail!("config must include at least one target");
    }
    Ok(config)
}
