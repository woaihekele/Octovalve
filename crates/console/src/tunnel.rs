pub(crate) struct TargetRuntime {
    pub(crate) name: String,
    pub(crate) ssh: Option<String>,
    pub(crate) ssh_args: Vec<String>,
    pub(crate) ssh_password: Option<String>,
    pub(crate) control_remote_addr: String,
    pub(crate) control_local_bind: Option<String>,
    pub(crate) control_local_port: Option<u16>,
    pub(crate) control_local_addr: Option<String>,
}

impl TargetRuntime {
    pub(crate) fn connect_addr(&self) -> String {
        self.control_local_addr
            .clone()
            .unwrap_or_else(|| self.control_remote_addr.clone())
    }
}
