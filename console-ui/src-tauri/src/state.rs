use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use tauri_plugin_shell::process::CommandChild;
use tokio::sync::mpsc;

use crate::types::{ProfilesFile, ProxyConfigStatus};

pub struct ConsoleSidecar {
    pub child: CommandChild,
    pub exited: Arc<AtomicBool>,
}

pub struct ConsoleSidecarState(pub Mutex<Option<ConsoleSidecar>>);
pub struct ConsoleStreamState(pub Mutex<bool>);
pub struct ProxyConfigState(pub Mutex<ProxyConfigStatus>);
pub struct ProfilesState(pub Mutex<ProfilesFile>);

pub struct TerminalSession {
    pub tx: mpsc::UnboundedSender<String>,
}

pub struct TerminalSessions(pub Mutex<HashMap<String, TerminalSession>>);

pub struct AppLogState {
    pub app_log: PathBuf,
}

pub struct AppLanguageState(pub Mutex<Option<String>>);
