use crate::config::{DaemonConfig, DaemonDefaults, TargetConfig};
use crate::ssh::{exit_master, forward_add, forward_cancel, spawn_master, SshTarget};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tokio::process::Child;
use tunnel_protocol::{ForwardPurpose, ForwardSpec, ForwardStatus};

const DEFAULT_REMOTE_ADDR: &str = "127.0.0.1:19307";
const DEFAULT_CONTROL_REMOTE_ADDR: &str = "127.0.0.1:19308";
const DEFAULT_BIND_HOST: &str = "127.0.0.1";
const DEFAULT_CONTROL_PORT_OFFSET: u16 = 100;

pub(crate) struct DaemonState {
    targets: HashMap<String, TargetState>,
    clients_last_seen: HashMap<String, SystemTime>,
}

struct TargetState {
    ssh: SshTarget,
    control_path: PathBuf,
    allowed_forwards: HashSet<ForwardSpec>,
    active_forwards: HashMap<ForwardSpec, ActiveForward>,
    master: Option<SshMaster>,
}

struct ActiveForward {
    clients: HashSet<String>,
}

struct SshMaster {
    child: Child,
}

struct ResolvedTarget {
    name: String,
    ssh: SshTarget,
    data_forward: ForwardSpec,
    control_forward: ForwardSpec,
}

impl DaemonState {
    pub(crate) fn build(config: DaemonConfig, control_dir: PathBuf) -> anyhow::Result<Self> {
        let defaults = config.defaults.unwrap_or_default();
        let mut targets = HashMap::new();
        let mut seen = HashSet::new();
        let mut local_addr_used = HashSet::new();

        for target in config.targets {
            if target.name.trim().is_empty() {
                anyhow::bail!("target name cannot be empty");
            }
            if seen.contains(&target.name) {
                anyhow::bail!("duplicate target name: {}", target.name);
            }
            seen.insert(target.name.clone());

            let Some(resolved) = resolve_target(&defaults, target)? else {
                continue;
            };

            let control_path = control_path_for(&control_dir, &resolved.name);
            let mut allowed_forwards = HashSet::new();
            for forward in [
                resolved.data_forward.clone(),
                resolved.control_forward.clone(),
            ] {
                let local_addr = forward.local_addr();
                if local_addr_used.contains(&local_addr) {
                    anyhow::bail!("duplicate local addr: {local_addr}");
                }
                local_addr_used.insert(local_addr);
                if !allowed_forwards.insert(forward) {
                    anyhow::bail!("duplicate forward in target {}", resolved.name);
                }
            }

            let state = TargetState {
                ssh: resolved.ssh,
                control_path,
                allowed_forwards,
                active_forwards: HashMap::new(),
                master: None,
            };
            targets.insert(resolved.name, state);
        }

        if targets.is_empty() {
            anyhow::bail!("no ssh targets available for tunnel-daemon");
        }

        Ok(Self {
            targets,
            clients_last_seen: HashMap::new(),
        })
    }

    pub(crate) async fn ensure_forward(
        &mut self,
        client_id: &str,
        forward: ForwardSpec,
    ) -> anyhow::Result<(String, bool)> {
        let target = self
            .targets
            .get_mut(&forward.target)
            .ok_or_else(|| anyhow::anyhow!("unknown target {}", forward.target))?;
        let result = target.ensure_forward(client_id, &forward).await;
        if result.is_ok() {
            self.clients_last_seen
                .insert(client_id.to_string(), SystemTime::now());
        }
        result
    }

    pub(crate) async fn release_forward(
        &mut self,
        client_id: &str,
        forward: ForwardSpec,
    ) -> anyhow::Result<bool> {
        let target = self
            .targets
            .get_mut(&forward.target)
            .ok_or_else(|| anyhow::anyhow!("unknown target {}", forward.target))?;
        let released = target.release_forward(client_id, &forward).await?;
        if released && !self.client_in_use(client_id) {
            self.clients_last_seen.remove(client_id);
        }
        Ok(released)
    }

    pub(crate) fn heartbeat(&mut self, client_id: &str) {
        self.clients_last_seen
            .insert(client_id.to_string(), SystemTime::now());
    }

    pub(crate) async fn cleanup_expired(&mut self, ttl: Duration) -> bool {
        let now = SystemTime::now();
        let mut expired = HashSet::new();
        for (client, last_seen) in &self.clients_last_seen {
            if now.duration_since(*last_seen).unwrap_or(Duration::ZERO) > ttl {
                expired.insert(client.clone());
            }
        }
        if !expired.is_empty() {
            for target in self.targets.values_mut() {
                target.cleanup_expired_clients(&expired).await;
            }
            for client in expired {
                self.clients_last_seen.remove(&client);
            }
        }
        self.clients_last_seen.is_empty() && !self.has_active_forwards()
    }

    pub(crate) fn list_forwards(&self) -> Vec<ForwardStatus> {
        let mut list = Vec::new();
        for target in self.targets.values() {
            for (forward, active) in &target.active_forwards {
                let mut clients: Vec<String> = active.clients.iter().cloned().collect();
                clients.sort();
                list.push(ForwardStatus {
                    forward: forward.clone(),
                    clients,
                });
            }
        }
        list
    }

    fn client_in_use(&self, client_id: &str) -> bool {
        self.targets.values().any(|target| {
            target
                .active_forwards
                .values()
                .any(|active| active.clients.contains(client_id))
        })
    }

    fn has_active_forwards(&self) -> bool {
        self.targets
            .values()
            .any(|target| !target.active_forwards.is_empty())
    }
}

impl TargetState {
    async fn ensure_forward(
        &mut self,
        client_id: &str,
        forward: &ForwardSpec,
    ) -> anyhow::Result<(String, bool)> {
        if !self.allowed_forwards.contains(forward) {
            anyhow::bail!("forward not allowed for target {}", forward.target);
        }
        self.ensure_master().await?;
        let mut reused = true;
        let entry = self.active_forwards.entry(forward.clone());
        let active = match entry {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                forward_add(&self.ssh, &self.control_path, forward).await?;
                reused = false;
                entry.insert(ActiveForward {
                    clients: HashSet::new(),
                })
            }
        };
        active.clients.insert(client_id.to_string());
        Ok((forward.local_addr(), reused))
    }

    async fn release_forward(
        &mut self,
        client_id: &str,
        forward: &ForwardSpec,
    ) -> anyhow::Result<bool> {
        let Some(active) = self.active_forwards.get_mut(forward) else {
            return Ok(false);
        };
        let removed = active.clients.remove(client_id);
        if active.clients.is_empty() {
            let _ = forward_cancel(&self.ssh, &self.control_path, forward).await;
            self.active_forwards.remove(forward);
        }
        if self.active_forwards.is_empty() {
            let _ = exit_master(&self.ssh, &self.control_path).await;
            self.master = None;
        }
        Ok(removed)
    }

    async fn cleanup_expired_clients(&mut self, expired: &HashSet<String>) {
        if expired.is_empty() {
            return;
        }
        let mut to_remove = Vec::new();
        for (forward, active) in &mut self.active_forwards {
            active.clients.retain(|client| !expired.contains(client));
            if active.clients.is_empty() {
                to_remove.push(forward.clone());
            }
        }
        for forward in to_remove {
            let _ = forward_cancel(&self.ssh, &self.control_path, &forward).await;
            self.active_forwards.remove(&forward);
        }
        if self.active_forwards.is_empty() {
            let _ = exit_master(&self.ssh, &self.control_path).await;
            self.master = None;
        }
    }

    async fn ensure_master(&mut self) -> anyhow::Result<()> {
        if let Some(master) = self.master.as_mut() {
            match master.child.try_wait() {
                Ok(None) => return Ok(()),
                Ok(Some(status)) => {
                    self.master = None;
                    self.active_forwards.clear();
                    tracing::warn!(error = %status, "ssh master exited, restarting");
                }
                Err(err) => {
                    self.master = None;
                    self.active_forwards.clear();
                    tracing::warn!(error = %err, "ssh master status check failed, restarting");
                }
            }
        }

        let child = spawn_master(&self.ssh, &self.control_path).await?;
        self.master = Some(SshMaster { child });
        Ok(())
    }
}

fn resolve_target(
    defaults: &DaemonDefaults,
    target: TargetConfig,
) -> anyhow::Result<Option<ResolvedTarget>> {
    let Some(ssh) = target.ssh else {
        tracing::warn!(target = %target.name, "skip target without ssh");
        return Ok(None);
    };
    if ssh.trim().is_empty() {
        tracing::warn!(target = %target.name, "skip target with empty ssh");
        return Ok(None);
    }
    let local_port = target
        .local_port
        .ok_or_else(|| anyhow::anyhow!("target {} missing local_port", target.name))?;
    if local_port == 0 {
        anyhow::bail!("target {} has invalid local_port 0", target.name);
    }

    let remote_addr = target
        .remote_addr
        .or_else(|| defaults.remote_addr.clone())
        .unwrap_or_else(|| DEFAULT_REMOTE_ADDR.to_string());

    let control_remote_addr = target
        .control_remote_addr
        .or_else(|| defaults.control_remote_addr.clone())
        .or_else(|| derive_control_addr(&remote_addr).ok())
        .unwrap_or_else(|| DEFAULT_CONTROL_REMOTE_ADDR.to_string());

    let data_bind = target
        .local_bind
        .clone()
        .or_else(|| defaults.local_bind.clone())
        .unwrap_or_else(|| DEFAULT_BIND_HOST.to_string());

    let control_bind = target
        .control_local_bind
        .or_else(|| target.local_bind.clone())
        .or_else(|| defaults.control_local_bind.clone())
        .or_else(|| defaults.local_bind.clone())
        .unwrap_or_else(|| DEFAULT_BIND_HOST.to_string());

    let offset = defaults
        .control_local_port_offset
        .unwrap_or(DEFAULT_CONTROL_PORT_OFFSET);
    let control_local_port = target
        .control_local_port
        .or_else(|| local_port.checked_add(offset));

    if control_local_port.is_none() {
        anyhow::bail!(
            "target {} requires control_local_port (or local_port + offset)",
            target.name
        );
    }

    let mut ssh_args = defaults.ssh_args.clone().unwrap_or_default();
    if let Some(extra) = target.ssh_args {
        ssh_args.extend(extra);
    }
    let ssh_password = target
        .ssh_password
        .or_else(|| defaults.ssh_password.clone());
    if ssh_password.is_some() {
        tracing::warn!(
            target = %target.name,
            "ssh_password is set; prefer SSH key auth (keyboard-interactive/2FA is not supported)"
        );
    }

    let ssh_target = SshTarget {
        ssh,
        ssh_args,
        ssh_password,
    };

    let data_forward = ForwardSpec {
        target: target.name.clone(),
        purpose: ForwardPurpose::Data,
        local_bind: data_bind,
        local_port,
        remote_addr,
    };

    let control_forward = ForwardSpec {
        target: target.name.clone(),
        purpose: ForwardPurpose::Control,
        local_bind: control_bind,
        local_port: control_local_port.unwrap(),
        remote_addr: control_remote_addr,
    };

    Ok(Some(ResolvedTarget {
        name: target.name,
        ssh: ssh_target,
        data_forward,
        control_forward,
    }))
}

fn control_path_for(control_dir: &Path, name: &str) -> PathBuf {
    let hash = hash_name(name);
    control_dir.join(format!("{hash}.sock"))
}

fn hash_name(name: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    hasher.finish()
}

fn derive_control_addr(remote_addr: &str) -> anyhow::Result<String> {
    let (host, port) = parse_host_port(remote_addr)?;
    let control_port = port.saturating_add(1);
    Ok(format!("{host}:{control_port}"))
}

fn parse_host_port(addr: &str) -> anyhow::Result<(String, u16)> {
    let (host, port) = addr
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("invalid address {addr}, expected host:port"))?;
    let port = port
        .parse::<u16>()
        .map_err(|_| anyhow::anyhow!("invalid port in address {addr}"))?;
    Ok((host.to_string(), port))
}
