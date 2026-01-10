use serde::{Deserialize, Serialize};

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
    EnsureForward {
        client_id: String,
        forward: ForwardSpec,
    },
    ReleaseForward {
        client_id: String,
        forward: ForwardSpec,
    },
    Heartbeat {
        client_id: String,
    },
    ListForwards,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TunnelResponse {
    EnsureForward { local_addr: String, reused: bool },
    ReleaseForward { released: bool },
    Ok,
    Forwards { items: Vec<ForwardStatus> },
    Error { message: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForwardStatus {
    pub forward: ForwardSpec,
    pub clients: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_spec_local_addr_formats() {
        let spec = ForwardSpec {
            target: "alpha".to_string(),
            purpose: ForwardPurpose::Data,
            local_bind: "127.0.0.1".to_string(),
            local_port: 19310,
            remote_addr: "10.0.0.1:22".to_string(),
        };
        assert_eq!(spec.local_addr(), "127.0.0.1:19310");
    }

    #[test]
    fn tunnel_request_serializes_with_snake_case_tag() {
        let spec = ForwardSpec {
            target: "alpha".to_string(),
            purpose: ForwardPurpose::Control,
            local_bind: "0.0.0.0".to_string(),
            local_port: 10001,
            remote_addr: "127.0.0.1:22".to_string(),
        };
        let request = TunnelRequest::EnsureForward {
            client_id: "client-1".to_string(),
            forward: spec,
        };

        let value = serde_json::to_value(&request).expect("serialize request");
        assert_eq!(
            value.get("type"),
            Some(&serde_json::Value::String("ensure_forward".into()))
        );
        assert_eq!(
            value
                .get("forward")
                .and_then(|forward| forward.get("purpose")),
            Some(&serde_json::Value::String("control".into()))
        );
    }
}
