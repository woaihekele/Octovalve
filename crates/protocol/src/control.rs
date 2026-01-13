use crate::{CommandMode, CommandStage, CommandStatus};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotCommonFields {
    pub id: String,
    pub client: String,
    pub target: String,
    pub peer: String,
    pub intent: String,
    pub mode: CommandMode,
    pub raw_command: String,
    pub pipeline: Vec<CommandStage>,
    pub cwd: Option<String>,
    pub timeout_ms: Option<u64>,
    pub max_output_bytes: Option<u64>,
    pub received_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestSnapshot {
    #[serde(flatten)]
    pub common: SnapshotCommonFields,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunningSnapshot {
    #[serde(flatten)]
    pub common: SnapshotCommonFields,
    pub queued_for_secs: u64,
    pub started_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResultSnapshot {
    pub id: String,
    pub status: CommandStatus,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub intent: String,
    pub mode: CommandMode,
    pub raw_command: String,
    pub pipeline: Vec<CommandStage>,
    pub cwd: Option<String>,
    pub peer: String,
    pub queued_for_secs: u64,
    pub finished_at_ms: u64,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceSnapshot {
    pub queue: Vec<RequestSnapshot>,
    pub running: Vec<RunningSnapshot>,
    pub history: Vec<ResultSnapshot>,
    pub last_result: Option<ResultSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ServiceEvent {
    QueueUpdated(Vec<RequestSnapshot>),
    RunningUpdated(Vec<RunningSnapshot>),
    ResultUpdated(ResultSnapshot),
    ConnectionsChanged,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlRequest {
    Snapshot,
    Approve { id: String },
    Deny { id: String },
    Cancel { id: String },
    Subscribe,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlResponse {
    Snapshot { snapshot: ServiceSnapshot },
    Ack { message: String },
    Error { message: String },
    Event { event: ServiceEvent },
}
