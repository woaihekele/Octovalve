use crate::state::TargetSpec;
use crate::AppState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use base64::engine::general_purpose::STANDARD as BASE64_ENGINE;
use base64::Engine;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::thread;
use tokio::sync::mpsc;
use tokio::task::spawn_blocking;
use tracing::{info, warn};

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
const DEFAULT_TERM: &str = "xterm-256color";
const ASKPASS_SCRIPT: &str = "#!/bin/sh\nprintf '%s' \"$OCTOVALVE_SSH_PASS\"\n";

#[derive(Debug, Deserialize)]
pub(crate) struct TerminalQuery {
    cols: Option<u16>,
    rows: Option<u16>,
    term: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TerminalRequest {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
    Close,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TerminalResponse {
    Ready { cols: u16, rows: u16, term: String },
    Output { data: String },
    Exit { code: Option<i32> },
    Error { message: String },
}

struct TerminalTarget {
    name: String,
    ssh: String,
    ssh_args: Vec<String>,
    ssh_password: Option<String>,
}

impl TerminalTarget {
    fn from_spec(spec: TargetSpec) -> Option<Self> {
        let ssh = spec.ssh?.trim().to_string();
        if ssh.is_empty() {
            return None;
        }
        Some(Self {
            name: spec.name,
            ssh,
            ssh_args: spec.ssh_args,
            ssh_password: spec.ssh_password,
        })
    }
}

#[derive(Clone)]
struct TerminalConfig {
    cols: u16,
    rows: u16,
    term: String,
}

enum TerminalOutput {
    Data(Vec<u8>),
    Exit(Option<i32>),
    Error(String),
}

enum TerminalAction {
    Continue,
    Close,
}

pub(crate) async fn terminal_ws_handler(
    ws: WebSocketUpgrade,
    Path(name): Path<String>,
    Query(query): Query<TerminalQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let spec = {
        let guard = state.state.read().await;
        guard.target_spec(&name)
    };
    let Some(spec) = spec else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(target) = TerminalTarget::from_spec(spec) else {
        return StatusCode::BAD_REQUEST.into_response();
    };

    let cols = query.cols.unwrap_or(DEFAULT_COLS).max(1);
    let rows = query.rows.unwrap_or(DEFAULT_ROWS).max(1);
    let term = query
        .term
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_TERM.to_string());
    let config = TerminalConfig { cols, rows, term };

    ws.on_upgrade(move |socket| handle_terminal(socket, target, config))
}

async fn handle_terminal(mut socket: WebSocket, target: TerminalTarget, config: TerminalConfig) {
    let pair = match native_pty_system().openpty(PtySize {
        rows: config.rows,
        cols: config.cols,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(pair) => pair,
        Err(err) => {
            let _ = send_response(
                &mut socket,
                TerminalResponse::Error {
                    message: format!("failed to open pty: {err}"),
                },
            )
            .await;
            return;
        }
    };

    let mut cmd = CommandBuilder::new("ssh");
    for arg in &target.ssh_args {
        cmd.arg(arg);
    }
    cmd.arg("-tt");
    cmd.arg(&target.ssh);
    cmd.env("TERM", &config.term);
    if let Some(password) = target.ssh_password.as_deref() {
        if let Err(err) = configure_askpass(&mut cmd, password) {
            let _ = send_response(
                &mut socket,
                TerminalResponse::Error {
                    message: format!("failed to configure ssh password: {err}"),
                },
            )
            .await;
            return;
        }
    }

    let mut child = match pair.slave.spawn_command(cmd) {
        Ok(child) => child,
        Err(err) => {
            let _ = send_response(
                &mut socket,
                TerminalResponse::Error {
                    message: format!("failed to spawn ssh: {err}"),
                },
            )
            .await;
            return;
        }
    };

    let mut master = pair.master;
    let (output_tx, mut output_rx) = mpsc::unbounded_channel::<TerminalOutput>();
    let (input_tx, input_rx) = std_mpsc::channel::<Vec<u8>>();

    if let Ok(reader) = master.try_clone_reader() {
        let output_tx = output_tx.clone();
        thread::spawn(move || read_pty_loop(reader, output_tx));
    } else {
        let _ = output_tx.send(TerminalOutput::Error(
            "failed to clone pty reader".to_string(),
        ));
    }

    match master.take_writer() {
        Ok(writer) => {
            let output_tx = output_tx.clone();
            thread::spawn(move || write_pty_loop(writer, input_rx, output_tx));
        }
        Err(err) => {
            let _ = output_tx.send(TerminalOutput::Error(format!(
                "failed to take pty writer: {err}"
            )));
        }
    }

    let (exit_tx, mut exit_rx) = tokio::sync::oneshot::channel();
    spawn_blocking(move || {
        let _ = child.wait();
        let _ = exit_tx.send(None);
    });

    let _ = send_response(
        &mut socket,
        TerminalResponse::Ready {
            cols: config.cols,
            rows: config.rows,
            term: config.term.clone(),
        },
    )
    .await;
    info!(target = %target.name, "terminal session started");

    loop {
        tokio::select! {
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => match handle_request(&text, &input_tx, &mut master) {
                        Ok(TerminalAction::Continue) => {}
                        Ok(TerminalAction::Close) => break,
                        Err(err) => {
                            warn!(target = %target.name, error = %err, "terminal request error");
                        }
                    },
                    Some(Ok(Message::Binary(_))) => {}
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(err)) => {
                        warn!(target = %target.name, error = %err, "terminal websocket error");
                        break;
                    }
                }
            }
            Some(output) = output_rx.recv() => {
                match output {
                    TerminalOutput::Data(bytes) => {
                        let response = TerminalResponse::Output { data: BASE64_ENGINE.encode(bytes) };
                        if send_response(&mut socket, response).await.is_err() {
                            break;
                        }
                    }
                    TerminalOutput::Exit(code) => {
                        let _ = send_response(&mut socket, TerminalResponse::Exit { code }).await;
                        break;
                    }
                    TerminalOutput::Error(message) => {
                        let _ = send_response(&mut socket, TerminalResponse::Error { message }).await;
                        break;
                    }
                }
            }
            code = &mut exit_rx => {
                let response = TerminalResponse::Exit { code: code.ok().flatten() };
                let _ = send_response(&mut socket, response).await;
                break;
            }
        }
    }

    info!(target = %target.name, "terminal session closed");
}

fn handle_request(
    text: &str,
    input_tx: &std_mpsc::Sender<Vec<u8>>,
    master: &mut Box<dyn portable_pty::MasterPty + Send>,
) -> anyhow::Result<TerminalAction> {
    let request: TerminalRequest = serde_json::from_str(text)?;
    match request {
        TerminalRequest::Input { data } => {
            let bytes = BASE64_ENGINE.decode(data)?;
            let _ = input_tx.send(bytes);
        }
        TerminalRequest::Resize { cols, rows } => {
            let cols = cols.max(1);
            let rows = rows.max(1);
            master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })?;
        }
        TerminalRequest::Close => {
            return Ok(TerminalAction::Close);
        }
    }
    Ok(TerminalAction::Continue)
}

fn read_pty_loop(
    mut reader: Box<dyn Read + Send>,
    output_tx: mpsc::UnboundedSender<TerminalOutput>,
) {
    let mut buffer = [0u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => {
                let _ = output_tx.send(TerminalOutput::Exit(None));
                break;
            }
            Ok(n) => {
                let _ = output_tx.send(TerminalOutput::Data(buffer[..n].to_vec()));
            }
            Err(err) => {
                let _ = output_tx.send(TerminalOutput::Error(format!("pty read failed: {err}")));
                break;
            }
        }
    }
}

fn write_pty_loop(
    mut writer: Box<dyn Write + Send>,
    input_rx: std_mpsc::Receiver<Vec<u8>>,
    output_tx: mpsc::UnboundedSender<TerminalOutput>,
) {
    while let Ok(chunk) = input_rx.recv() {
        if writer.write_all(&chunk).is_err() {
            let _ = output_tx.send(TerminalOutput::Error("pty write failed".to_string()));
            break;
        }
        let _ = writer.flush();
    }
}

async fn send_response(
    socket: &mut WebSocket,
    response: TerminalResponse,
) -> Result<(), axum::Error> {
    let payload = match serde_json::to_string(&response) {
        Ok(payload) => payload,
        Err(err) => {
            warn!(error = %err, "failed to serialize terminal response");
            return Ok(());
        }
    };
    socket.send(Message::Text(payload)).await
}

fn configure_askpass(cmd: &mut CommandBuilder, password: &str) -> anyhow::Result<()> {
    let script = ensure_askpass_script()?;
    cmd.env("OCTOVALVE_SSH_PASS", password);
    cmd.env("SSH_ASKPASS", script.to_string_lossy().to_string());
    cmd.env("SSH_ASKPASS_REQUIRE", "force");
    cmd.env("DISPLAY", "1");
    Ok(())
}

fn ensure_askpass_script() -> anyhow::Result<PathBuf> {
    let home = std::env::var("HOME")?;
    let dir = PathBuf::from(home).join(".octovalve");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("ssh-askpass.sh");
    let mut needs_write = true;
    if let Ok(existing) = std::fs::read(&path) {
        if existing == ASKPASS_SCRIPT.as_bytes() {
            needs_write = false;
        }
    }
    if needs_write {
        std::fs::write(&path, ASKPASS_SCRIPT.as_bytes())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)?.permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&path, perms)?;
    }
    Ok(path)
}
