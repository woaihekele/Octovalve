use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tauri::{AppHandle, Manager};

use protocol::config::ProxyConfig;

use crate::services::config::DEFAULT_BROKER_CONFIG;
use crate::services::profiles::{
    current_profile_entry, profile_broker_path, resolve_broker_config_path,
};
use crate::types::{ProfilesFile, ProxyConfigStatus, StartupCheckResult};

#[derive(Debug, Deserialize)]
struct BrokerConfig {
    #[serde(default)]
    whitelist: WhitelistConfig,
    #[serde(default)]
    limits: LimitsConfig,
    #[serde(default = "default_auto_approve_allowed")]
    auto_approve_allowed: bool,
}

#[derive(Debug, Deserialize, Default)]
struct WhitelistConfig {
    #[serde(default)]
    allowed: Vec<String>,
    #[serde(default)]
    denied: Vec<String>,
    #[serde(default)]
    arg_rules: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct LimitsConfig {
    timeout_secs: u64,
    max_output_bytes: u64,
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

pub fn validate_startup_config(
    app: &AppHandle,
    proxy_status: &ProxyConfigStatus,
    profiles: &ProfilesFile,
) -> Result<StartupCheckResult, String> {
    let proxy_path = PathBuf::from(proxy_status.path.clone());
    let mut errors = Vec::new();
    let mut needs_setup = false;
    let broker_path = resolve_broker_path(app, profiles, &proxy_path)
        .unwrap_or_else(|_| default_broker_path(app));

    if !proxy_status.present {
        needs_setup = true;
        errors.push(format!(
            "未找到本地配置：{}，请参考 {} 创建并修改。",
            proxy_status.path, proxy_status.example_path
        ));
        return Ok(build_result(errors, needs_setup, proxy_path, broker_path));
    }

    match fs::read_to_string(&proxy_path) {
        Ok(raw) => match toml::from_str::<ProxyConfig>(&raw) {
            Ok(config) => {
                if is_example_config(&config) {
                    needs_setup = true;
                    errors.push("检测到示例配置，请先修改默认环境配置。".to_string());
                }
                errors.extend(validate_proxy_config(&config));
            }
            Err(err) => errors.push(format_toml_error("本地配置", &proxy_path, &raw, err)),
        },
        Err(err) => errors.push(format!(
            "读取本地配置失败：{} ({})",
            proxy_path.display(),
            err
        )),
    }

    if let Err(err) = validate_broker_config(&broker_path) {
        errors.push(err);
    }

    Ok(build_result(errors, needs_setup, proxy_path, broker_path))
}

fn build_result(
    errors: Vec<String>,
    needs_setup: bool,
    proxy_path: PathBuf,
    broker_path: PathBuf,
) -> StartupCheckResult {
    StartupCheckResult {
        ok: errors.is_empty(),
        needs_setup,
        errors,
        proxy_path: proxy_path.to_string_lossy().to_string(),
        broker_path: broker_path.to_string_lossy().to_string(),
    }
}

fn resolve_broker_path(
    app: &AppHandle,
    profiles: &ProfilesFile,
    proxy_path: &Path,
) -> Result<PathBuf, String> {
    if let Ok(current) = current_profile_entry(profiles) {
        if !current.broker_path.trim().is_empty() {
            return profile_broker_path(app, &current);
        }
    }
    let config_dir = app.path().app_config_dir().map_err(|err| err.to_string())?;
    let resolved = resolve_broker_config_path(app, proxy_path, &config_dir, Some(profiles))?;
    Ok(resolved.path)
}

fn default_broker_path(app: &AppHandle) -> PathBuf {
    let dir = app.path().app_config_dir().ok();
    dir.unwrap_or_else(|| PathBuf::from("."))
        .join("remote-broker-config.toml")
}

fn validate_broker_config(path: &Path) -> Result<(), String> {
    let raw = if path.exists() {
        fs::read_to_string(path)
            .map_err(|err| format!("读取远端配置失败：{} ({})", path.display(), err))?
    } else {
        DEFAULT_BROKER_CONFIG.to_string()
    };
    toml::from_str::<BrokerConfig>(&raw)
        .map_err(|err| format_toml_error("远端配置", path, &raw, err))?;
    Ok(())
}

fn validate_proxy_config(config: &ProxyConfig) -> Vec<String> {
    let mut errors = Vec::new();
    if config.targets.is_empty() {
        errors.push("本地配置必须至少包含一个 target。".to_string());
        return errors;
    }

    let defaults = config.defaults.as_ref();
    let mut seen = HashSet::new();
    let mut control_addr_seen = HashSet::new();

    for target in &config.targets {
        let name = target.name.trim();
        if name.is_empty() {
            errors.push("target name 不能为空。".to_string());
            continue;
        }
        if !seen.insert(name.to_string()) {
            errors.push(format!("target 名称重复：{name}"));
        }
        if let Some(ssh) = target.ssh.as_deref() {
            if ssh.trim().is_empty() {
                errors.push(format!("target {name} 的 ssh 不能为空。"));
            }
            if ssh.split_whitespace().count() > 1 {
                errors.push(format!(
                    "target {name} 的 ssh 只能是单一目标，参数请放到 ssh_args。"
                ));
            }
        }
        if target.ssh.is_some() && target.local_port.is_none() {
            errors.push(format!(
                "target {name} 缺少 local_port（ssh 模式必须填写）。"
            ));
        }
        if target.ssh.is_some() {
            let control_local_port = protocol::config::control_local_port(defaults, target);
            if control_local_port.is_none() {
                errors.push(format!(
                    "target {name} 缺少 control_local_port（可用 local_port + offset 自动生成）。"
                ));
            } else if let Some(addr) =
                protocol::config::control_local_addr(defaults, target, control_local_port)
            {
                if !control_addr_seen.insert(addr.clone()) {
                    errors.push(format!("target {name} 的 control_local_addr 重复：{addr}"));
                }
            }
        }
    }

    if let Some(default_target) = config.default_target.as_ref() {
        if !seen.contains(default_target) {
            errors.push(format!("default_target 未找到对应目标：{default_target}"));
        }
    }

    errors
}

fn is_example_config(config: &ProxyConfig) -> bool {
    if config
        .default_target
        .as_deref()
        .map(|value| value.eq_ignore_ascii_case("example"))
        .unwrap_or(false)
    {
        return true;
    }
    for target in &config.targets {
        let name = target.name.trim();
        if name.eq_ignore_ascii_case("example") {
            return true;
        }
        if let Some(ssh) = target.ssh.as_deref() {
            if ssh.trim() == "user@host" {
                return true;
            }
        }
        let desc = target.desc.to_lowercase();
        if desc.contains("example target") || desc.contains("replace with your machine") {
            return true;
        }
    }
    false
}

fn format_toml_error(label: &str, path: &Path, raw: &str, err: toml::de::Error) -> String {
    let detail = err.to_string();
    if let Some(span) = err.span() {
        if let Some((line, col)) = line_col_from_span(raw, span) {
            return format!(
                "{}解析失败（{}:{}）：{} ({})",
                label,
                line,
                col,
                detail,
                path.display()
            );
        }
    }
    format!("{}解析失败：{} ({})", label, detail, path.display())
}

fn line_col_from_span(input: &str, span: std::ops::Range<usize>) -> Option<(usize, usize)> {
    let mut start = span.start.min(input.len());
    while start > 0 && !input.is_char_boundary(start) {
        start -= 1;
    }
    let mut line = 1usize;
    let mut col = 1usize;
    for ch in input[..start].chars() {
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    Some((line, col))
}
