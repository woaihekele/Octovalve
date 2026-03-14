use regex::Regex;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use protocol::{CommandRequest, CommandStage};

const FIND_PIPE_MANUAL_REVIEW_TOKENS: [&str; 7] =
    ["delete", "rm", "dd", "cat", "tee", "del", "remove-item"];
const FIND_EXEC_FLAGS: [&str; 4] = ["-exec", "-execdir", "-ok", "-okdir"];

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct PolicyConfig {
    #[serde(default)]
    pub(crate) whitelist: WhitelistConfig,
    #[serde(default)]
    pub(crate) limits: LimitsConfig,
    #[serde(default = "default_auto_approve_allowed")]
    pub(crate) auto_approve_allowed: bool,
    #[serde(default)]
    pub(crate) ai_readonly_review: AiReadonlyReviewConfig,
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
    #[serde(default = "default_find_pipe_manual_review_tokens")]
    pub(crate) find_pipe_manual_review_tokens: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LimitsConfig {
    pub(crate) timeout_secs: u64,
    pub(crate) max_output_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AiReadonlyReviewConfig {
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default = "default_ai_endpoint")]
    pub(crate) endpoint: String,
    #[serde(default = "default_ai_model")]
    pub(crate) model: String,
    #[serde(default = "default_ai_api_key_env")]
    pub(crate) api_key_env: String,
    #[serde(default = "default_ai_timeout_ms")]
    pub(crate) timeout_ms: u64,
    #[serde(default = "default_ai_min_confidence")]
    pub(crate) min_confidence: f64,
    #[serde(default = "default_ai_max_command_chars")]
    pub(crate) max_command_chars: usize,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            timeout_secs: 30,
            max_output_bytes: 1024 * 1024,
        }
    }
}

impl Default for AiReadonlyReviewConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: default_ai_endpoint(),
            model: default_ai_model(),
            api_key_env: default_ai_api_key_env(),
            timeout_ms: default_ai_timeout_ms(),
            min_confidence: default_ai_min_confidence(),
            max_command_chars: default_ai_max_command_chars(),
        }
    }
}

fn default_auto_approve_allowed() -> bool {
    true
}

fn default_ai_endpoint() -> String {
    "https://api.openai.com/v1/chat/completions".to_string()
}

fn default_ai_model() -> String {
    "gpt-4.1-mini".to_string()
}

fn default_ai_api_key_env() -> String {
    "OPENAI_API_KEY".to_string()
}

fn default_ai_timeout_ms() -> u64 {
    2_500
}

fn default_ai_min_confidence() -> f64 {
    0.90
}

fn default_ai_max_command_chars() -> usize {
    4_000
}

fn default_find_pipe_manual_review_tokens() -> Vec<String> {
    FIND_PIPE_MANUAL_REVIEW_TOKENS
        .iter()
        .map(|token| (*token).to_string())
        .collect()
}

#[derive(Debug, Clone)]
pub(crate) struct Whitelist {
    #[allow(dead_code)]
    allowed: HashSet<String>,
    denied: HashSet<String>,
    #[allow(dead_code)]
    arg_rules: HashMap<String, Regex>,
    find_pipe_manual_review_tokens: HashSet<String>,
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
            find_pipe_manual_review_tokens: config
                .find_pipe_manual_review_tokens
                .iter()
                .map(|value| value.trim().to_ascii_lowercase())
                .filter(|value| !value.is_empty())
                .collect(),
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
        let allow_ok = request
            .pipeline
            .iter()
            .all(|stage| self.validate_allow(stage).is_ok());
        if !allow_ok {
            return false;
        }
        if self.requires_manual_review_for_find_pipeline(&request.pipeline) {
            return false;
        }
        true
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

    fn requires_manual_review_for_find_pipeline(&self, pipeline: &[CommandStage]) -> bool {
        if self.find_pipe_manual_review_tokens.is_empty() {
            return false;
        }
        let has_find = pipeline
            .iter()
            .filter_map(|stage| stage.command())
            .filter_map(command_basename)
            .any(|command| command.eq_ignore_ascii_case("find"));
        if !has_find {
            return false;
        }

        let has_risky_token = pipeline.iter().any(|stage| {
            stage
                .argv
                .iter()
                .map(String::as_str)
                .filter_map(command_basename)
                .any(|token| {
                    self.find_pipe_manual_review_tokens
                        .contains(&token.to_ascii_lowercase())
                })
        });
        if !has_risky_token {
            return false;
        }

        // 规则一：find 通过管道串联高风险命令 => 转人工审核。
        if pipeline.len() > 1 {
            return true;
        }

        // 规则二：find -exec/-execdir/-ok/-okdir 直接触发高风险命令 => 转人工审核。
        pipeline.iter().any(stage_has_find_exec_flag)
    }
}

fn command_basename<'a>(command: &'a str) -> Option<&'a str> {
    std::path::Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
}

fn stage_has_find_exec_flag(stage: &CommandStage) -> bool {
    if !stage
        .command()
        .and_then(command_basename)
        .is_some_and(|command| command.eq_ignore_ascii_case("find"))
    {
        return false;
    }
    stage.argv.iter().skip(1).any(|arg| {
        FIND_EXEC_FLAGS
            .iter()
            .any(|flag| arg.eq_ignore_ascii_case(flag))
    })
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

    fn request_with_pipeline(stages: &[&[&str]]) -> CommandRequest {
        CommandRequest {
            id: "req-1".to_string(),
            client: "client".to_string(),
            target: "target".to_string(),
            intent: "intent".to_string(),
            mode: protocol::CommandMode::Shell,
            raw_command: stages
                .iter()
                .map(|stage| stage.join(" "))
                .collect::<Vec<_>>()
                .join(" | "),
            cwd: None,
            env: None,
            timeout_ms: None,
            max_output_bytes: None,
            pipeline: stages
                .iter()
                .map(|stage| CommandStage {
                    argv: stage.iter().map(|value| (*value).to_string()).collect(),
                })
                .collect(),
        }
    }

    #[test]
    fn allows_exact_command() {
        let config = WhitelistConfig {
            allowed: vec!["ls".to_string()],
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            arg_rules,
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
        };
        let whitelist = Whitelist::from_config(&config).expect("whitelist");
        let stage = CommandStage {
            argv: vec!["/bin/rm".to_string(), "-rf".to_string(), "/".to_string()],
        };
        assert!(whitelist.validate_deny(&stage).is_err());
    }

    #[test]
    fn find_pipeline_with_rm_requires_manual_review() {
        let config = WhitelistConfig {
            allowed: vec!["find".to_string(), "xargs".to_string()],
            find_pipe_manual_review_tokens: vec![
                "delete".to_string(),
                "rm".to_string(),
                "dd".to_string(),
                "cat".to_string(),
                "tee".to_string(),
            ],
            ..Default::default()
        };
        let whitelist = Whitelist::from_config(&config).expect("whitelist");
        let request =
            request_with_pipeline(&[&["find", ".", "-name", "*.tmp"], &["xargs", "rm", "-f"]]);

        assert!(!whitelist.allows_request(&request));
        assert!(deny_message(&whitelist, &request).is_none());
    }

    #[test]
    fn find_pipeline_without_risky_tokens_can_auto_approve() {
        let config = WhitelistConfig {
            allowed: vec!["find".to_string(), "grep".to_string()],
            find_pipe_manual_review_tokens: vec!["rm".to_string()],
            ..Default::default()
        };
        let whitelist = Whitelist::from_config(&config).expect("whitelist");
        let request = request_with_pipeline(&[&["find", ".", "-name", "*.rs"], &["grep", "mod"]]);

        assert!(whitelist.allows_request(&request));
    }

    #[test]
    fn find_exec_with_rm_requires_manual_review() {
        let config = WhitelistConfig {
            allowed: vec!["find".to_string()],
            find_pipe_manual_review_tokens: vec!["rm".to_string()],
            ..Default::default()
        };
        let whitelist = Whitelist::from_config(&config).expect("whitelist");
        let request = request_with_pipeline(&[&[
            "find", ".", "-name", "*.tmp", "-exec", "rm", "-f", "{}", ";",
        ]]);

        assert!(!whitelist.allows_request(&request));
        assert!(deny_message(&whitelist, &request).is_none());
    }

    #[test]
    fn find_exec_without_risky_tokens_can_auto_approve() {
        let config = WhitelistConfig {
            allowed: vec!["find".to_string()],
            find_pipe_manual_review_tokens: vec!["rm".to_string(), "tee".to_string()],
            ..Default::default()
        };
        let whitelist = Whitelist::from_config(&config).expect("whitelist");
        let request =
            request_with_pipeline(&[&["find", ".", "-name", "*.rs", "-exec", "echo", "{}", ";"]]);

        assert!(whitelist.allows_request(&request));
    }

    #[test]
    fn serde_default_includes_find_pipe_manual_review_tokens() {
        let config: WhitelistConfig = toml::from_str("").expect("parse");
        assert!(config
            .find_pipe_manual_review_tokens
            .iter()
            .any(|token| token == "rm"));
    }

    #[test]
    fn policy_defaults_include_disabled_ai_review() {
        let config: PolicyConfig = toml::from_str("").expect("parse");
        assert!(!config.ai_readonly_review.enabled);
        assert_eq!(
            config.ai_readonly_review.endpoint,
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(config.ai_readonly_review.model, "gpt-4.1-mini");
    }
}
