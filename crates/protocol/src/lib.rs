use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod config;
pub mod control;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandStage {
    pub argv: Vec<String>,
}

impl CommandStage {
    pub fn command(&self) -> Option<&str> {
        self.argv.first().map(String::as_str)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandMode {
    Shell,
    Argv,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandRequest {
    pub id: String,
    pub client: String,
    pub target: String,
    pub intent: String,
    pub mode: CommandMode,
    pub raw_command: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub max_output_bytes: Option<u64>,
    pub pipeline: Vec<CommandStage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    Approved,
    Denied,
    Error,
    Cancelled,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandResponse {
    pub id: String,
    pub status: CommandStatus,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub stdout: Option<String>,
    #[serde(default)]
    pub stderr: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

impl CommandResponse {
    pub fn denied(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            status: CommandStatus::Denied,
            exit_code: None,
            stdout: None,
            stderr: None,
            error: Some(message.into()),
        }
    }

    pub fn error(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            status: CommandStatus::Error,
            exit_code: None,
            stdout: None,
            stderr: None,
            error: Some(message.into()),
        }
    }

    pub fn cancelled(
        id: impl Into<String>,
        exit_code: Option<i32>,
        stdout: Option<String>,
        stderr: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            status: CommandStatus::Cancelled,
            exit_code,
            stdout,
            stderr,
            error: Some("cancelled by operator".to_string()),
        }
    }

    pub fn completed(
        id: impl Into<String>,
        exit_code: i32,
        stdout: Option<String>,
        stderr: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            status: CommandStatus::Completed,
            exit_code: Some(exit_code),
            stdout,
            stderr,
            error: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_request_roundtrip() {
        let request = CommandRequest {
            id: "req-1".to_string(),
            client: "octovalve-proxy".to_string(),
            target: "default".to_string(),
            intent: "list files".to_string(),
            mode: CommandMode::Shell,
            raw_command: "echo hello".to_string(),
            cwd: Some("/tmp".to_string()),
            env: Some(BTreeMap::from([("LANG".to_string(), "C".to_string())])),
            timeout_ms: Some(5000),
            max_output_bytes: Some(1024),
            pipeline: vec![CommandStage {
                argv: vec!["echo".to_string(), "hello".to_string()],
            }],
        };

        let json = serde_json::to_string(&request).expect("serialize");
        let decoded: CommandRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(request, decoded);
    }

    #[test]
    fn command_response_roundtrip() {
        let response =
            CommandResponse::completed("req-2", 0, Some("ok".to_string()), Some(String::new()));
        let json = serde_json::to_string(&response).expect("serialize");
        let decoded: CommandResponse = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(response, decoded);
    }
}
