use std::collections::BTreeMap;
use std::ffi::OsStr;

pub(crate) trait CommandArgs {
    fn arg<S: AsRef<OsStr>>(&mut self, arg: S) -> &mut Self;
}

impl CommandArgs for std::process::Command {
    fn arg<S: AsRef<OsStr>>(&mut self, arg: S) -> &mut Self {
        std::process::Command::arg(self, arg)
    }
}

impl CommandArgs for tokio::process::Command {
    fn arg<S: AsRef<OsStr>>(&mut self, arg: S) -> &mut Self {
        tokio::process::Command::arg(self, arg)
    }
}

pub(crate) fn apply_ssh_options<C: CommandArgs>(cmd: &mut C, has_password: bool) {
    cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
    cmd.arg("-o").arg("ConnectTimeout=10");
    if !has_password {
        cmd.arg("-o").arg("BatchMode=yes");
    }
}

pub(crate) fn build_env_prefix(pairs: &BTreeMap<String, String>) -> String {
    let mut parts = Vec::new();
    for (key, value) in pairs {
        if key.trim().is_empty() {
            continue;
        }
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        parts.push(format!("{key}={}", shell_escape(value)));
    }
    parts.join(" ")
}

pub(crate) fn env_locale(key: &str) -> Option<String> {
    let value = std::env::var(key).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(crate) fn env_language_locale(key: &str) -> Option<String> {
    let value = std::env::var(key).ok()?;
    let trimmed = value.trim().to_lowercase();
    if trimmed.starts_with("zh") {
        return Some("zh_CN.utf8".to_string());
    }
    if trimmed.starts_with("en") {
        return Some("en_US.utf8".to_string());
    }
    None
}

pub(crate) fn shell_escape(value: &str) -> String {
    let mut escaped = String::from("'");
    for ch in value.chars() {
        if ch == '\'' {
            escaped.push_str("'\"'\"'");
        } else {
            escaped.push(ch);
        }
    }
    escaped.push('\'');
    escaped
}
