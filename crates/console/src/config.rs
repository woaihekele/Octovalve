use anyhow::Context;
pub(crate) use protocol::config::{
    ProxyConfig as ConsoleConfig, ProxyDefaults as ConsoleDefaults, TargetConfig,
};
use std::path::PathBuf;

const DEFAULT_REMOTE_ADDR: &str = "127.0.0.1:19307";
const DEFAULT_CONTROL_REMOTE_ADDR: &str = "127.0.0.1:19308";

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
