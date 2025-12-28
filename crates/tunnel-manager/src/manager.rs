use crate::ssh::{exit_master, forward_add, forward_cancel, spawn_master, SshTarget};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Child;
use tokio::sync::Mutex;
use tunnel_protocol::ForwardSpec;

const CONTROL_SOCKET_WAIT_TIMEOUT: Duration = Duration::from_secs(2);
const CONTROL_SOCKET_WAIT_INTERVAL: Duration = Duration::from_millis(50);

pub struct TunnelTargetSpec {
    pub name: String,
    pub ssh: String,
    pub ssh_args: Vec<String>,
    pub ssh_password: Option<String>,
    pub allowed_forwards: Vec<ForwardSpec>,
}

pub struct TunnelManager {
    state: Mutex<ManagerState>,
}

struct ManagerState {
    targets: HashMap<String, TargetState>,
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

impl TunnelManager {
    pub fn new(targets: Vec<TunnelTargetSpec>, control_dir: PathBuf) -> anyhow::Result<Self> {
        if targets.is_empty() {
            anyhow::bail!("no ssh targets available for tunnel manager");
        }
        std::fs::create_dir_all(&control_dir)
            .map_err(|err| anyhow::anyhow!("failed to create {}: {}", control_dir.display(), err))?;

        let mut state_targets = HashMap::new();
        let mut seen = HashSet::new();
        let mut local_addr_used = HashSet::new();

        for target in targets {
            if target.name.trim().is_empty() {
                anyhow::bail!("target name cannot be empty");
            }
            if seen.contains(&target.name) {
                anyhow::bail!("duplicate target name: {}", target.name);
            }
            seen.insert(target.name.clone());

            let control_path = control_path_for(&control_dir, &target.name);
            let mut allowed_forwards = HashSet::new();
            for forward in target.allowed_forwards {
                let local_addr = forward.local_addr();
                if local_addr_used.contains(&local_addr) {
                    anyhow::bail!("duplicate local addr: {local_addr}");
                }
                local_addr_used.insert(local_addr);
                if !allowed_forwards.insert(forward) {
                    anyhow::bail!("duplicate forward in target {}", target.name);
                }
            }

            let state = TargetState {
                ssh: SshTarget {
                    ssh: target.ssh,
                    ssh_args: target.ssh_args,
                    ssh_password: target.ssh_password,
                },
                control_path,
                allowed_forwards,
                active_forwards: HashMap::new(),
                master: None,
            };
            state_targets.insert(target.name, state);
        }

        Ok(Self {
            state: Mutex::new(ManagerState {
                targets: state_targets,
            }),
        })
    }

    pub async fn ensure_forward(
        &self,
        client_id: &str,
        forward: &ForwardSpec,
    ) -> anyhow::Result<String> {
        let mut state = self.state.lock().await;
        let target = state
            .targets
            .get_mut(&forward.target)
            .ok_or_else(|| anyhow::anyhow!("unknown target {}", forward.target))?;
        target.ensure_forward(client_id, forward).await?;
        Ok(forward.local_addr())
    }

    pub async fn release_forward(
        &self,
        client_id: &str,
        forward: &ForwardSpec,
    ) -> anyhow::Result<bool> {
        let mut state = self.state.lock().await;
        let target = state
            .targets
            .get_mut(&forward.target)
            .ok_or_else(|| anyhow::anyhow!("unknown target {}", forward.target))?;
        target.release_forward(client_id, forward).await
    }

    pub async fn shutdown(&self) {
        let mut state = self.state.lock().await;
        for target in state.targets.values_mut() {
            target.shutdown().await;
        }
    }
}

impl TargetState {
    async fn ensure_forward(
        &mut self,
        client_id: &str,
        forward: &ForwardSpec,
    ) -> anyhow::Result<()> {
        if !self.allowed_forwards.contains(forward) {
            anyhow::bail!("forward not allowed for target {}", forward.target);
        }
        self.ensure_master().await?;
        let entry = self.active_forwards.entry(forward.clone());
        let active = match entry {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                forward_add(&self.ssh, &self.control_path, forward).await?;
                entry.insert(ActiveForward {
                    clients: HashSet::new(),
                })
            }
        };
        active.clients.insert(client_id.to_string());
        Ok(())
    }

    async fn release_forward(
        &mut self,
        client_id: &str,
        forward: &ForwardSpec,
    ) -> anyhow::Result<bool> {
        let removed = if let Some(active) = self.active_forwards.get_mut(forward) {
            let removed = active.clients.remove(client_id);
            if active.clients.is_empty() {
                let _ = forward_cancel(&self.ssh, &self.control_path, forward).await;
                self.active_forwards.remove(forward);
            }
            removed
        } else {
            false
        };
        if self.active_forwards.is_empty() {
            self.shutdown_master().await;
        }
        Ok(removed)
    }

    async fn shutdown(&mut self) {
        let forwards: Vec<ForwardSpec> = self.active_forwards.keys().cloned().collect();
        for forward in forwards {
            let _ = forward_cancel(&self.ssh, &self.control_path, &forward).await;
            self.active_forwards.remove(&forward);
        }
        self.shutdown_master().await;
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

        let mut child = spawn_master(&self.ssh, &self.control_path).await?;
        if let Err(err) = wait_for_control_socket(&self.control_path).await {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(err);
        }
        self.master = Some(SshMaster { child });
        Ok(())
    }

    async fn shutdown_master(&mut self) {
        let _ = exit_master(&self.ssh, &self.control_path).await;
        if let Some(mut master) = self.master.take() {
            match master.child.try_wait() {
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => {
                    let _ = master.child.kill().await;
                    let _ = master.child.wait().await;
                }
            }
        }
    }
}

async fn wait_for_control_socket(control_path: &Path) -> anyhow::Result<()> {
    let start = std::time::Instant::now();
    loop {
        if control_path.exists() {
            return Ok(());
        }
        if start.elapsed() >= CONTROL_SOCKET_WAIT_TIMEOUT {
            anyhow::bail!(
                "control socket not ready after {}ms",
                CONTROL_SOCKET_WAIT_TIMEOUT.as_millis()
            );
        }
        tokio::time::sleep(CONTROL_SOCKET_WAIT_INTERVAL).await;
    }
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
