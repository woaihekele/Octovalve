use serde::{Deserialize, Serialize};

pub const DEFAULT_TUNNEL_DAEMON_ADDR: &str = "127.0.0.1:19310";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ForwardPurpose {
    Data,
    Control,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ForwardSpec {
    pub target: String,
    pub purpose: ForwardPurpose,
    pub local_bind: String,
    pub local_port: u16,
    pub remote_addr: String,
}

impl ForwardSpec {
    pub fn local_addr(&self) -> String {
        format!("{}:{}", self.local_bind, self.local_port)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TunnelRequest {
    EnsureForward { client_id: String, forward: ForwardSpec },
    ReleaseForward { client_id: String, forward: ForwardSpec },
    ListForwards,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TunnelResponse {
    EnsureForward { local_addr: String, reused: bool },
    ReleaseForward { released: bool },
    Forwards { items: Vec<ForwardStatus> },
    Error { message: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForwardStatus {
    pub forward: ForwardSpec,
    pub clients: Vec<String>,
}
