use crate::shared::snapshot::{RequestSnapshot, ResultSnapshot, ServiceSnapshot};
use protocol::{CommandRequest, CommandResponse};
use std::time::{Instant, SystemTime};
use tokio::sync::oneshot;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub(crate) enum ServiceEvent {
    QueueUpdated(Vec<RequestSnapshot>),
    ResultUpdated(ResultSnapshot),
    ConnectionsChanged,
}

pub(crate) enum ServiceCommand {
    Approve(String),
    Deny(String),
    Snapshot(oneshot::Sender<ServiceSnapshot>),
}

pub(crate) enum ServerEvent {
    ConnectionOpened,
    ConnectionClosed,
    Request(PendingRequest),
}

pub(crate) struct PendingRequest {
    pub(crate) request: CommandRequest,
    pub(crate) peer: String,
    pub(crate) received_at: SystemTime,
    pub(crate) queued_at: Instant,
    pub(crate) respond_to: oneshot::Sender<CommandResponse>,
}
