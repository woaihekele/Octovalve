use protocol::{CommandMode, CommandStage, CommandStatus};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct RequestSnapshot {
    pub(crate) id: String,
    pub(crate) client: String,
    pub(crate) target: String,
    pub(crate) peer: String,
    pub(crate) intent: String,
    pub(crate) mode: CommandMode,
    pub(crate) raw_command: String,
    pub(crate) pipeline: Vec<CommandStage>,
    pub(crate) cwd: Option<String>,
    pub(crate) timeout_ms: Option<u64>,
    pub(crate) max_output_bytes: Option<u64>,
    pub(crate) received_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ResultSnapshot {
    pub(crate) id: String,
    pub(crate) status: CommandStatus,
    pub(crate) exit_code: Option<i32>,
    pub(crate) error: Option<String>,
    pub(crate) intent: String,
    pub(crate) mode: CommandMode,
    pub(crate) raw_command: String,
    pub(crate) pipeline: Vec<CommandStage>,
    pub(crate) cwd: Option<String>,
    pub(crate) peer: String,
    pub(crate) queued_for_secs: u64,
    pub(crate) finished_at_ms: u64,
    pub(crate) stdout: Option<String>,
    pub(crate) stderr: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ServiceSnapshot {
    pub(crate) queue: Vec<RequestSnapshot>,
    pub(crate) history: Vec<ResultSnapshot>,
    pub(crate) last_result: Option<ResultSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub(crate) enum ServiceEvent {
    QueueUpdated(Vec<RequestSnapshot>),
    ResultUpdated(ResultSnapshot),
    ConnectionsChanged,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ControlRequest {
    Snapshot,
    Approve { id: String },
    Deny { id: String },
    Subscribe,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ControlResponse {
    Snapshot { snapshot: ServiceSnapshot },
    Ack { message: String },
    Error { message: String },
    Event { event: ServiceEvent },
}
