use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub whitelist: WhitelistConfig,
    #[serde(default)]
    pub limits: LimitsConfig,
    #[serde(default = "default_auto_approve_allowed")]
    pub auto_approve_allowed: bool,
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config = toml::from_str(&content)?;
        Ok(config)
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct WhitelistConfig {
    #[serde(default)]
    pub allowed: Vec<String>,
    #[serde(default)]
    pub denied: Vec<String>,
    #[serde(default)]
    pub arg_rules: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LimitsConfig {
    pub timeout_secs: u64,
    pub max_output_bytes: u64,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            timeout_secs: 30,
            max_output_bytes: 1024 * 1024,
        }
    }
}

fn default_auto_approve_allowed() -> bool {
    true
}
