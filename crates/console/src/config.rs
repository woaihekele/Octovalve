use anyhow::Context;
use serde::Deserialize;
use std::path::PathBuf;

const DEFAULT_REMOTE_ADDR: &str = "127.0.0.1:19307";
const DEFAULT_CONTROL_REMOTE_ADDR: &str = "127.0.0.1:19308";

#[derive(Debug, Deserialize)]
pub(crate) struct ConsoleConfig {
    pub(crate) default_target: Option<String>,
    pub(crate) defaults: Option<ConsoleDefaults>,
    pub(crate) targets: Vec<TargetConfig>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConsoleDefaults {
    pub(crate) remote_addr: Option<String>,
    pub(crate) local_bind: Option<String>,
    pub(crate) ssh_args: Option<Vec<String>>,
    pub(crate) ssh_password: Option<String>,
    pub(crate) control_remote_addr: Option<String>,
    pub(crate) control_local_bind: Option<String>,
    pub(crate) control_local_port_offset: Option<u16>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TargetConfig {
    pub(crate) name: String,
    pub(crate) desc: String,
    pub(crate) hostname: Option<String>,
    pub(crate) ip: Option<String>,
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

impl Default for ConsoleDefaults {
    fn default() -> Self {
        Self {
            remote_addr: None,
            local_bind: None,
            ssh_args: None,
            ssh_password: None,
            control_remote_addr: None,
            control_local_bind: None,
            control_local_port_offset: None,
        }
    }
}

pub(crate) fn load_console_config(path: &PathBuf) -> anyhow::Result<ConsoleConfig> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    let config: ConsoleConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse config {}", path.display()))?;
    if config.targets.is_empty() {
        anyhow::bail!("config must include at least one target");
    }
    Ok(config)
}

pub(crate) fn default_remote_addr() -> String {
    DEFAULT_REMOTE_ADDR.to_string()
}

pub(crate) fn default_control_remote_addr() -> String {
    DEFAULT_CONTROL_REMOTE_ADDR.to_string()
}
