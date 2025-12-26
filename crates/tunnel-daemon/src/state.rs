use crate::config::{DaemonConfig, DaemonDefaults};
use crate::ssh::{exit_master, forward_add, forward_cancel, spawn_master, SshTarget};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tokio::process::Child;
use tunnel_protocol::{ForwardSpec, ForwardStatus};

pub(crate) struct DaemonState {
    targets: HashMap<String, TargetState>,
}

struct TargetState {
    ssh: SshTarget,
    control_path: PathBuf,
    allowed_forwards: HashSet<ForwardSpec>,
    active_forwards: HashMap<ForwardSpec, ActiveForward>,
    master: Option<SshMaster>,
    last_used: Option<SystemTime>,
}

struct ActiveForward {
    clients: HashSet<String>,
}

struct SshMaster {
    child: Child,
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

            if target.forwards.is_empty() {
                anyhow::bail!("target {} must include at least one forward", target.name);
            }

            let ssh_args = merge_ssh_args(&defaults, target.ssh_args);
            let ssh_password = target
                .ssh_password
                .or_else(|| defaults.ssh_password.clone());
            let ssh = SshTarget {
                ssh: target.ssh,
                ssh_args,
                ssh_password,
            };
            let control_path = control_path_for(&control_dir, &target.name);

            let mut allowed_forwards = HashSet::new();
            for forward in target.forwards {
                if forward.local_port == 0 {
                    anyhow::bail!("target {} has invalid local_port 0", target.name);
                }
                let local_bind = forward
                    .local_bind
                    .or_else(|| defaults.local_bind.clone())
                    .unwrap_or_else(|| "127.0.0.1".to_string());
                let forward_spec = ForwardSpec {
                    target: target.name.clone(),
                    purpose: forward.purpose,
                    local_bind: local_bind.clone(),
                    local_port: forward.local_port,
                    remote_addr: forward.remote_addr,
                };
                let local_addr = forward_spec.local_addr();
                if local_addr_used.contains(&local_addr) {
                    anyhow::bail!("duplicate local addr: {local_addr}");
                }
                local_addr_used.insert(local_addr);
                if !allowed_forwards.insert(forward_spec) {
                    anyhow::bail!("duplicate forward in target {}", target.name);
                }
            }

            let state = TargetState {
                ssh,
                control_path,
                allowed_forwards,
                active_forwards: HashMap::new(),
                master: None,
                last_used: None,
            };
            targets.insert(target.name.clone(), state);
        }

        Ok(Self { targets })
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
        target.ensure_forward(client_id, &forward).await
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
        target.release_forward(client_id, &forward).await
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
        self.last_used = Some(SystemTime::now());
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

fn merge_ssh_args(defaults: &DaemonDefaults, target: Option<Vec<String>>) -> Vec<String> {
    let mut args = defaults.ssh_args.clone().unwrap_or_default();
    if let Some(extra) = target {
        args.extend(extra);
    }
    args
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
