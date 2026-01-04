use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

pub struct ResolvedBrokerConfig {
    pub path: PathBuf,
    pub source: String,
}

#[derive(Clone, Serialize)]
pub struct StartupCheckResult {
    pub ok: bool,
    pub errors: Vec<String>,
    pub proxy_path: String,
    pub broker_path: String,
}
