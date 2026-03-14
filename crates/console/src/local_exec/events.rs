use protocol::{CommandRequest, CommandResponse};
use std::time::{Instant, SystemTime};
use tokio::sync::oneshot;

pub(crate) enum ServerEvent {
    ConnectionOpened,
    ConnectionClosed,
    Request(PendingRequest),
    AutoApproveRequest(PendingRequest),
}

pub(crate) struct PendingRequest {
    pub(crate) request: CommandRequest,
    pub(crate) target_host: Option<String>,
    pub(crate) target_desc: Option<String>,
    pub(crate) peer: String,
    pub(crate) received_at: SystemTime,
    pub(crate) queued_at: Instant,
    pub(crate) aggressive_mode: bool,
    pub(crate) respond_to: oneshot::Sender<CommandResponse>,
}
