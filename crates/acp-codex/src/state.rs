use std::collections::VecDeque;

use codex_protocol::ConversationId;
use tokio::sync::oneshot;

#[derive(Default)]
pub(crate) struct AcpState {
    pub(crate) session_id: Option<String>,
    pub(crate) conversation_id: Option<ConversationId>,
    pub(crate) pending_prompt_ids: VecDeque<u64>,
    pub(crate) session_id_waiters: Vec<oneshot::Sender<String>>,
    pub(crate) app_server_initialized: bool,
    pub(crate) saw_message_delta: bool,
    pub(crate) saw_reasoning_delta: bool,
}
