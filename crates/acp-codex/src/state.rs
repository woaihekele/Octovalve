use std::collections::VecDeque;

#[derive(Default)]
pub(crate) struct AcpState {
    pub(crate) session_id: Option<String>,
    pub(crate) thread_id: Option<String>,
    pub(crate) active_turn_id: Option<String>,
    pub(crate) pending_prompt_ids: VecDeque<u64>,
    pub(crate) app_server_initialized: bool,
    pub(crate) saw_message_delta: bool,
    pub(crate) saw_reasoning_delta: bool,
    pub(crate) retry_count: u32,
    pub(crate) retry_exhausted: bool,
}
