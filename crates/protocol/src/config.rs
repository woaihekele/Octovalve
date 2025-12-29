use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ProxyConfig {
    pub default_target: Option<String>,
    pub defaults: Option<ProxyDefaults>,
    pub targets: Vec<TargetConfig>,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
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
