use regex::Regex;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use protocol::{CommandRequest, CommandStage};

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct PolicyConfig {
    #[serde(default)]
    pub(crate) whitelist: WhitelistConfig,
    #[serde(default)]
    pub(crate) limits: LimitsConfig,
    #[serde(default = "default_auto_approve_allowed")]
    pub(crate) auto_approve_allowed: bool,
}

impl PolicyConfig {
    pub(crate) fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config = toml::from_str(&content)?;
        Ok(config)
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct WhitelistConfig {
    #[serde(default)]
    pub(crate) allowed: Vec<String>,
    #[serde(default)]
    pub(crate) denied: Vec<String>,
    #[serde(default)]
    pub(crate) arg_rules: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LimitsConfig {
    pub(crate) timeout_secs: u64,
    pub(crate) max_output_bytes: u64,
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

#[derive(Debug, Clone)]
pub(crate) struct Whitelist {
    #[allow(dead_code)]
    allowed: HashSet<String>,
    denied: HashSet<String>,
    #[allow(dead_code)]
    arg_rules: HashMap<String, Regex>,
}

impl Whitelist {
    pub(crate) fn from_config(config: &WhitelistConfig) -> anyhow::Result<Self> {
        let mut arg_rules = HashMap::new();
        for (command, pattern) in &config.arg_rules {
            let regex = Regex::new(pattern)
                .map_err(|err| anyhow::anyhow!("invalid regex for {command}: {err}"))?;
            arg_rules.insert(command.to_string(), regex);
        }
        Ok(Self {
            allowed: config.allowed.iter().cloned().collect(),
            denied: config.denied.iter().cloned().collect(),
            arg_rules,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn validate_allow(&self, stage: &CommandStage) -> Result<(), String> {
        let command = stage.command().ok_or_else(|| "empty command".to_string())?;
        if !self.is_allowed(command) {
            return Err(format!("command not allowed: {command}"));
        }

        let rule = self.arg_rules.get(command).or_else(|| {
            self.basename(command)
                .and_then(|name| self.arg_rules.get(name))
        });

        if let Some(rule) = rule {
            for arg in stage.argv.iter().skip(1) {
                if !rule.is_match(arg) {
                    return Err(format!("argument rejected: {arg}"));
                }
            }
        }

        Ok(())
    }

    pub(crate) fn validate_deny(&self, stage: &CommandStage) -> Result<(), String> {
        let command = stage.command().ok_or_else(|| "empty command".to_string())?;
        if self.is_denied(command) {
            return Err(format!("command denied: {command}"));
        }
        Ok(())
    }

    pub(crate) fn allows_request(&self, request: &CommandRequest) -> bool {
        if self.allowed.is_empty() {
            return false;
        }
        if request.pipeline.is_empty() {
            return false;
        }
        request
            .pipeline
            .iter()
            .all(|stage| self.validate_allow(stage).is_ok())
    }

    #[allow(dead_code)]
    fn is_allowed(&self, command: &str) -> bool {
        if self.allowed.contains(command) {
            return true;
        }
        if let Some(name) = self.basename(command) {
            return self.allowed.contains(name);
        }
        false
    }

    fn is_denied(&self, command: &str) -> bool {
        if self.denied.contains(command) {
            return true;
        }
        if let Some(name) = self.basename(command) {
            return self.denied.contains(name);
        }
        false
    }

    fn basename<'a>(&self, command: &'a str) -> Option<&'a str> {
        std::path::Path::new(command)
            .file_name()
            .and_then(|name| name.to_str())
    }
}

pub(crate) fn deny_message(whitelist: &Whitelist, request: &CommandRequest) -> Option<String> {
    for stage in &request.pipeline {
        if let Err(message) = whitelist.validate_deny(stage) {
            return Some(message);
        }
    }
    None
}

pub(crate) fn request_summary(request: &CommandRequest) -> String {
    let pipeline = format_pipeline(&request.pipeline);
    if pipeline.is_empty() {
        request.raw_command.clone()
    } else {
        pipeline
    }
}

fn format_pipeline(pipeline: &[CommandStage]) -> String {
    pipeline
        .iter()
        .map(|stage| stage.argv.join(" "))
        .collect::<Vec<_>>()
        .join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_exact_command() {
        let config = WhitelistConfig {
            allowed: vec!["ls".to_string()],
            denied: Vec::new(),
            arg_rules: BTreeMap::new(),
        };
        let whitelist = Whitelist::from_config(&config).expect("whitelist");
        let stage = CommandStage {
            argv: vec!["ls".to_string(), "-l".to_string()],
        };
        assert!(whitelist.validate_allow(&stage).is_ok());
    }

    #[test]
    fn allows_basename_match() {
        let config = WhitelistConfig {
            allowed: vec!["grep".to_string()],
            denied: Vec::new(),
            arg_rules: BTreeMap::new(),
        };
        let whitelist = Whitelist::from_config(&config).expect("whitelist");
        let stage = CommandStage {
            argv: vec!["/usr/bin/grep".to_string(), "foo".to_string()],
        };
        assert!(whitelist.validate_allow(&stage).is_ok());
    }

    #[test]
    fn rejects_disallowed_command() {
        let config = WhitelistConfig {
            allowed: vec!["ls".to_string()],
            denied: Vec::new(),
            arg_rules: BTreeMap::new(),
        };
        let whitelist = Whitelist::from_config(&config).expect("whitelist");
        let stage = CommandStage {
            argv: vec!["rm".to_string(), "-rf".to_string(), "/".to_string()],
        };
        assert!(whitelist.validate_allow(&stage).is_err());
    }

    #[test]
    fn enforces_argument_rules() {
        let mut arg_rules = BTreeMap::new();
        arg_rules.insert("grep".to_string(), "^[A-Za-z0-9_\\.-]+$".to_string());
        let config = WhitelistConfig {
            allowed: vec!["grep".to_string()],
            denied: Vec::new(),
            arg_rules,
        };
        let whitelist = Whitelist::from_config(&config).expect("whitelist");
        let ok_stage = CommandStage {
            argv: vec!["grep".to_string(), "needle".to_string()],
        };
        let bad_stage = CommandStage {
            argv: vec!["grep".to_string(), "bad$".to_string()],
        };
        assert!(whitelist.validate_allow(&ok_stage).is_ok());
        assert!(whitelist.validate_allow(&bad_stage).is_err());
    }

    #[test]
    fn rejects_denied_command() {
        let config = WhitelistConfig {
            allowed: vec!["ls".to_string()],
            denied: vec!["rm".to_string()],
            arg_rules: BTreeMap::new(),
        };
        let whitelist = Whitelist::from_config(&config).expect("whitelist");
        let stage = CommandStage {
            argv: vec!["rm".to_string(), "-rf".to_string(), "/".to_string()],
        };
        assert!(whitelist.validate_deny(&stage).is_err());
    }

    #[test]
    fn rejects_denied_basename() {
        let config = WhitelistConfig {
            allowed: vec!["/bin/ls".to_string()],
            denied: vec!["rm".to_string()],
            arg_rules: BTreeMap::new(),
        };
        let whitelist = Whitelist::from_config(&config).expect("whitelist");
        let stage = CommandStage {
            argv: vec!["/bin/rm".to_string(), "-rf".to_string(), "/".to_string()],
        };
        assert!(whitelist.validate_deny(&stage).is_err());
    }
}
