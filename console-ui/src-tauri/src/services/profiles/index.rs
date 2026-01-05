use std::fs;
use std::path::Path;

use crate::services::config::write_config_file;
use crate::types::{ProfileRecord, ProfileSummary, ProfilesFile, ProfilesStatus};

pub fn validate_profile_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("环境名称不能为空".to_string());
    }
    if trimmed.len() > 48 {
        return Err("环境名称最长支持 48 个字符".to_string());
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err("环境名称仅支持字母、数字、- 或 _".to_string());
    }
    Ok(())
}

pub fn profiles_status(data: &ProfilesFile) -> ProfilesStatus {
    ProfilesStatus {
        current: data.current.clone(),
        profiles: data
            .profiles
            .iter()
            .map(|profile| ProfileSummary {
                name: profile.name.clone(),
            })
            .collect(),
    }
}

pub fn current_profile_entry(data: &ProfilesFile) -> Result<ProfileRecord, String> {
    data.profiles
        .iter()
        .find(|profile| profile.name == data.current)
        .cloned()
        .ok_or_else(|| "current profile missing in profiles list".to_string())
}

pub fn profile_entry_by_name(data: &ProfilesFile, name: &str) -> Result<ProfileRecord, String> {
    data.profiles
        .iter()
        .find(|profile| profile.name == name)
        .cloned()
        .ok_or_else(|| format!("未找到环境 {}", name))
}

pub(crate) fn load_profiles_file(path: &Path) -> Result<ProfilesFile, String> {
    let raw = fs::read_to_string(path).map_err(|err| err.to_string())?;
    let parsed: ProfilesFile = toml::from_str(&raw).map_err(|err| err.to_string())?;
    if parsed.profiles.is_empty() {
        return Err("profiles.toml 必须至少包含一个环境".to_string());
    }
    let mut seen = std::collections::HashSet::new();
    for profile in &parsed.profiles {
        if profile.name.trim().is_empty() {
            return Err("环境名称不能为空".to_string());
        }
        if !seen.insert(profile.name.clone()) {
            return Err(format!("重复的环境名称：{}", profile.name));
        }
    }
    Ok(parsed)
}

pub(crate) fn write_profiles_file(path: &Path, data: &ProfilesFile) -> Result<(), String> {
    let content = toml::to_string_pretty(data).map_err(|err| err.to_string())?;
    write_config_file(path, &content)
}
