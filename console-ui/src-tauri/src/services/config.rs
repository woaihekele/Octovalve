use std::fs;
use std::path::Path;

use crate::types::ConfigFilePayload;

pub const DEFAULT_PROXY_EXAMPLE: &str =
    include_str!("../../resources/local-proxy-config.toml.example");
pub const DEFAULT_BROKER_CONFIG: &str = include_str!("../../../../config/config.toml");

pub fn ensure_file(path: &Path, contents: &str) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    fs::write(path, contents).map_err(|err| err.to_string())
}

pub fn read_config_file(path: &Path, fallback: Option<&str>) -> Result<ConfigFilePayload, String> {
    let exists = path.exists();
    let content = if exists {
        fs::read_to_string(path).map_err(|err| err.to_string())?
    } else {
        fallback.unwrap_or_default().to_string()
    };
    Ok(ConfigFilePayload {
        path: path.to_string_lossy().to_string(),
        exists,
        content,
    })
}

pub fn write_config_file(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    fs::write(path, content).map_err(|err| err.to_string())
}
