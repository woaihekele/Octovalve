use std::time::Instant;

#[derive(Clone)]
pub(crate) struct RequestView {
    pub(crate) id: String,
    pub(crate) summary: String,
    pub(crate) client: String,
    pub(crate) target: String,
    pub(crate) peer: String,
    pub(crate) intent: String,
    pub(crate) mode: String,
    pub(crate) command: String,
    pub(crate) pipeline: Option<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) timeout_ms: Option<u64>,
    pub(crate) max_output_bytes: Option<u64>,
    pub(crate) queued_at: Instant,
}

#[derive(Clone)]
pub(crate) struct ResultView {
    pub(crate) id: String,
    pub(crate) status: String,
    pub(crate) summary: String,
    pub(crate) command: String,
    pub(crate) client: String,
    pub(crate) target: String,
    pub(crate) peer: String,
    pub(crate) intent: String,
    pub(crate) mode: String,
    pub(crate) pipeline: Option<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) timeout_ms: Option<u64>,
    pub(crate) max_output_bytes: Option<u64>,
    pub(crate) queued_for_secs: u64,
    pub(crate) finished_at_ms: u64,
    pub(crate) exit_code: Option<i32>,
    pub(crate) stdout: Option<String>,
    pub(crate) stderr: Option<String>,
}
