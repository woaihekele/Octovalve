use crate::shared::dto::{RequestView, ResultView};
use protocol::{CommandRequest, CommandResponse};
use std::time::{Instant, SystemTime};
use tokio::sync::oneshot;

pub(crate) enum ServiceEvent {
    QueueUpdated(Vec<RequestView>),
    ResultUpdated(ResultView),
    ConnectionsChanged(usize),
}

pub(crate) enum ServiceCommand {
    Approve(String),
    Deny(String),
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
