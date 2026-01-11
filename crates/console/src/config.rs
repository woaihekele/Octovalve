use anyhow::Context;
pub(crate) use protocol::config::{
    ProxyConfig as ConsoleConfig, ProxyDefaults as ConsoleDefaults, TargetConfig,
};
use std::path::PathBuf;
use tracing::warn;

pub(crate) fn load_console_config(path: &PathBuf) -> anyhow::Result<ConsoleConfig> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    let config: ConsoleConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse config {}", path.display()))?;
    if config.targets.is_empty() {
        warn!("config has no targets; console will start without workers");
    }
    Ok(config)
}
