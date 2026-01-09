use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use protocol::config::{ProxyDefaults, TargetConfig};

#[derive(Clone, Serialize)]
pub struct ProxyConfigStatus {
    pub present: bool,
    pub path: String,
    pub example_path: String,
}

#[derive(Clone, Serialize)]
pub struct ConfigFilePayload {
    pub path: String,
    pub exists: bool,
    pub content: String,
}

#[derive(Deserialize)]
pub struct ProxyConfigOverrides {
    pub broker_config_path: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct ProxyConfigEditor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broker_config_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defaults: Option<ProxyDefaults>,
    #[serde(default)]
    pub targets: Vec<TargetConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BrokerConfigEditor {
    #[serde(default)]
    pub whitelist: BrokerWhitelistConfig,
    #[serde(default)]
    pub limits: BrokerLimitsConfig,
    #[serde(default = "default_auto_approve_allowed")]
    pub auto_approve_allowed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct BrokerWhitelistConfig {
    #[serde(default)]
    pub allowed: Vec<String>,
    #[serde(default)]
    pub denied: Vec<String>,
    #[serde(default)]
    pub arg_rules: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BrokerLimitsConfig {
    pub timeout_secs: u64,
    pub max_output_bytes: u64,
}

impl Default for BrokerLimitsConfig {
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

pub struct ResolvedBrokerConfig {
    pub path: PathBuf,
    pub source: String,
}

#[derive(Clone, Serialize)]
pub struct StartupCheckResult {
    pub ok: bool,
    pub needs_setup: bool,
    pub errors: Vec<String>,
    pub proxy_path: String,
    pub broker_path: String,
}
