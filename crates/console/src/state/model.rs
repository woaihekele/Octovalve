use serde::Serialize;

pub(crate) enum ControlCommand {
    Approve(String),
    Deny(String),
    Cancel(String),
    ForceCancel(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TargetStatus {
    Ready,
    Down,
}

#[derive(Clone, Debug)]
pub(crate) struct TargetSpec {
    pub(crate) name: String,
    pub(crate) desc: String,
    pub(crate) ssh: Option<String>,
    pub(crate) ssh_args: Vec<String>,
    pub(crate) ssh_password: Option<String>,
    pub(crate) terminal_locale: Option<String>,
    pub(crate) tty: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct TargetInfo {
    pub(crate) name: String,
    pub(crate) desc: String,
    pub(crate) ssh: Option<String>,
    pub(crate) status: TargetStatus,
    pub(crate) pending_count: usize,
    pub(crate) last_seen: Option<String>,
    pub(crate) last_error: Option<String>,
    pub(crate) terminal_available: bool,
    pub(crate) is_default: bool,
}
