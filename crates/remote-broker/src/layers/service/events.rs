pub(crate) use protocol::control::ServiceEvent;
use protocol::control::ServiceSnapshot;
use protocol::{CommandRequest, CommandResponse};
use std::time::{Instant, SystemTime};
use tokio::sync::oneshot;

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
