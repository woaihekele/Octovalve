use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProxyConfig {
    pub default_target: Option<String>,
    pub defaults: Option<ProxyDefaults>,
    pub targets: Vec<TargetConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProxyDefaults {
    pub timeout_ms: Option<u64>,
    pub max_output_bytes: Option<u64>,
    pub local_bind: Option<String>,
    pub remote_addr: Option<String>,
    pub ssh_args: Option<Vec<String>>,
    pub ssh_password: Option<String>,
    pub terminal_locale: Option<String>,
    pub control_remote_addr: Option<String>,
    pub control_local_bind: Option<String>,
    pub control_local_port_offset: Option<u16>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TargetConfig {
    pub name: String,
    pub desc: String,
    pub hostname: Option<String>,
    pub ip: Option<String>,
    pub ssh: Option<String>,
    pub remote_addr: Option<String>,
    pub local_port: Option<u16>,
    pub local_bind: Option<String>,
    pub ssh_args: Option<Vec<String>>,
    pub ssh_password: Option<String>,
    pub terminal_locale: Option<String>,
    pub control_remote_addr: Option<String>,
    pub control_local_port: Option<u16>,
    pub control_local_bind: Option<String>,
}

impl Default for ProxyDefaults {
    fn default() -> Self {
        Self {
            timeout_ms: None,
            max_output_bytes: None,
            local_bind: None,
            remote_addr: None,
            ssh_args: None,
            ssh_password: None,
            terminal_locale: None,
            control_remote_addr: None,
            control_local_bind: None,
            control_local_port_offset: None,
        }
    }
}

pub const DEFAULT_BIND_HOST: &str = "127.0.0.1";
pub const DEFAULT_CONTROL_PORT_OFFSET: u16 = 100;

pub fn derive_control_addr(remote_addr: &str) -> Result<String, String> {
    let (host, port) = parse_host_port(remote_addr)?;
    let control_port = port.saturating_add(1);
    Ok(format!("{host}:{control_port}"))
}

pub fn control_local_port(defaults: Option<&ProxyDefaults>, target: &TargetConfig) -> Option<u16> {
    let offset = defaults
        .and_then(|value| value.control_local_port_offset)
        .unwrap_or(DEFAULT_CONTROL_PORT_OFFSET);
    target
        .control_local_port
        .or_else(|| target.local_port.and_then(|port| port.checked_add(offset)))
}

pub fn control_local_bind(defaults: Option<&ProxyDefaults>, target: &TargetConfig) -> String {
    target
        .control_local_bind
        .clone()
        .or_else(|| target.local_bind.clone())
        .or_else(|| defaults.and_then(|value| value.control_local_bind.clone()))
        .or_else(|| defaults.and_then(|value| value.local_bind.clone()))
        .unwrap_or_else(|| DEFAULT_BIND_HOST.to_string())
}

pub fn control_local_addr(
    defaults: Option<&ProxyDefaults>,
    target: &TargetConfig,
    port: Option<u16>,
) -> Option<String> {
    let port = port?;
    let bind = control_local_bind(defaults, target);
    Some(format!("{bind}:{port}"))
}

fn parse_host_port(addr: &str) -> Result<(String, u16), String> {
    let (host, port) = addr
        .rsplit_once(':')
        .ok_or_else(|| format!("invalid address {addr}, expected host:port"))?;
    let port = port
        .parse::<u16>()
        .map_err(|_| format!("invalid port in address {addr}"))?;
    Ok((host.to_string(), port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_control_addr_from_remote() {
        let addr = derive_control_addr("127.0.0.1:19307").expect("addr");
        assert_eq!(addr, "127.0.0.1:19308");
    }
}
