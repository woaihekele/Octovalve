mod config;
mod executor;
mod whitelist;

use crate::config::Config;
use crate::executor::execute_request;
use crate::whitelist::Whitelist;
use anyhow::Context;
use bytes::Bytes;
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEvent};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use futures_util::{SinkExt, StreamExt};
use protocol::{CommandMode, CommandRequest, CommandResponse, CommandStage, CommandStatus};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Terminal;
use serde::Serialize;
use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "remote-broker",
    version,
    about = "Remote command broker with approval TUI"
)]
struct Args {
    #[arg(long, default_value = "127.0.0.1:19307")]
    listen_addr: String,
    #[arg(long, default_value = "config.toml")]
    config: PathBuf,
    #[arg(long, default_value = "logs")]
    audit_dir: PathBuf,
    #[arg(long)]
    auto_approve: bool,
    #[arg(long, default_value_t = false)]
    log_to_stderr: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let _file_guard = init_tracing(&args.audit_dir, args.log_to_stderr)?;

    let config = Config::load(&args.config)
        .with_context(|| format!("failed to load config {}", args.config.display()))?;
    let whitelist = Arc::new(Whitelist::from_config(&config.whitelist)?);
    let limits = Arc::new(config.limits);
    let output_dir = Arc::new(args.audit_dir.join("requests"));
    std::fs::create_dir_all(&*output_dir)?;

    let listener = TcpListener::bind(&args.listen_addr)
        .await
        .with_context(|| format!("failed to bind {}", args.listen_addr))?;

    if args.auto_approve {
        run_headless(listener, whitelist, limits, output_dir).await?;
        return Ok(());
    }

    let (ui_tx, mut ui_rx) = mpsc::channel::<UiEvent>(128);
    spawn_accept_loop(
        listener,
        ui_tx.clone(),
        Arc::clone(&output_dir),
        Arc::clone(&whitelist),
    );

    let mut terminal = setup_terminal()?;
    let mut app = AppState::default();

    let tick_rate = Duration::from_millis(100);
    loop {
        while let Ok(event) = ui_rx.try_recv() {
            app.handle_event(event);
        }

        terminal.draw(|frame| draw_ui(frame, &mut app))?;

        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                if handle_key_event(
                    key,
                    &mut app,
                    ui_tx.clone(),
                    Arc::clone(&whitelist),
                    Arc::clone(&limits),
                    Arc::clone(&output_dir),
                ) {
                    break;
                }
            }
        }
    }

    restore_terminal(&mut terminal)?;
    Ok(())
}

fn spawn_accept_loop(
    listener: TcpListener,
    ui_tx: mpsc::Sender<UiEvent>,
    output_dir: Arc<PathBuf>,
    whitelist: Arc<Whitelist>,
) {
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let accept_tx = ui_tx.clone();
                    let output_dir = Arc::clone(&output_dir);
                    let whitelist = Arc::clone(&whitelist);
                    tokio::spawn(async move {
                        if let Err(err) =
                            handle_connection_tui(stream, addr, accept_tx, output_dir, whitelist)
                                .await
                        {
                            tracing::error!(error = %err, "connection handler failed");
                        }
                    });
                }
                Err(err) => {
                    tracing::error!(error = %err, "listener accept failed");
                }
            }
        }
    });
}

async fn run_headless(
    listener: TcpListener,
    whitelist: Arc<Whitelist>,
    limits: Arc<crate::config::LimitsConfig>,
    output_dir: Arc<PathBuf>,
) -> anyhow::Result<()> {
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let whitelist = Arc::clone(&whitelist);
                    let limits = Arc::clone(&limits);
                    let output_dir = Arc::clone(&output_dir);
                    tokio::spawn(async move {
                        if let Err(err) =
                            handle_connection_auto(stream, addr, whitelist, limits, output_dir)
                                .await
                        {
                            tracing::error!(error = %err, "connection handler failed");
                        }
                    });
                }
                Err(err) => {
                    tracing::error!(error = %err, "listener accept failed");
                }
            }
        }
    });

    tokio::signal::ctrl_c().await?;
    Ok(())
}

async fn handle_connection_tui(
    stream: TcpStream,
    addr: SocketAddr,
    ui_tx: mpsc::Sender<UiEvent>,
    output_dir: Arc<PathBuf>,
    whitelist: Arc<Whitelist>,
) -> anyhow::Result<()> {
    let _ = ui_tx.send(UiEvent::ConnectionOpened).await;
    let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
    while let Some(frame) = framed.next().await {
        let bytes = frame.context("frame read")?;
        let request: CommandRequest = match serde_json::from_slice(&bytes) {
            Ok(request) => request,
            Err(err) => {
                tracing::warn!(error = %err, "invalid request payload");
                let response = CommandResponse::error("invalid", "invalid request");
                let payload = serde_json::to_vec(&response)?;
                let _ = framed.send(Bytes::from(payload)).await;
                continue;
            }
        };

        tracing::info!(
            event = "request_received",
            id = %request.id,
            client = %request.client,
            peer = %addr,
            command = %request_summary(&request),
        );

        if let Some(message) = deny_message(&whitelist, &request) {
            tracing::info!(
                event = "request_denied_policy",
                id = %request.id,
                client = %request.client,
                peer = %addr,
                reason = %message,
            );
            let received_at = SystemTime::now();
            let record = RequestRecord::from_request(&request, &addr.to_string(), received_at);
            spawn_write_request_record_value(Arc::clone(&output_dir), record);
            let response =
                CommandResponse::denied(request.id.clone(), format!("denied by policy: {message}"));
            crate::executor::write_result_record(&output_dir, &response, Duration::from_secs(0))
                .await;
            let payload = serde_json::to_vec(&response)?;
            let _ = framed.send(Bytes::from(payload)).await;
            continue;
        }

        let (respond_to, response_rx) = oneshot::channel();
        let received_at = SystemTime::now();
        let pending = PendingRequest {
            request,
            peer: addr.to_string(),
            received_at,
            queued_at: Instant::now(),
            respond_to,
        };
        spawn_write_request_record(Arc::clone(&output_dir), &pending);
        if ui_tx.send(UiEvent::Request(pending)).await.is_err() {
            break;
        }

        match response_rx.await {
            Ok(response) => {
                let payload = serde_json::to_vec(&response)?;
                framed.send(Bytes::from(payload)).await?;
            }
            Err(_) => break,
        }
    }
    let _ = ui_tx.send(UiEvent::ConnectionClosed).await;
    Ok(())
}

async fn handle_connection_auto(
    stream: TcpStream,
    addr: SocketAddr,
    whitelist: Arc<Whitelist>,
    limits: Arc<crate::config::LimitsConfig>,
    output_dir: Arc<PathBuf>,
) -> anyhow::Result<()> {
    let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
    while let Some(frame) = framed.next().await {
        let bytes = frame.context("frame read")?;
        let request: CommandRequest = match serde_json::from_slice(&bytes) {
            Ok(request) => request,
            Err(err) => {
                tracing::warn!(error = %err, "invalid request payload");
                let response = CommandResponse::error("invalid", "invalid request");
                let payload = serde_json::to_vec(&response)?;
                let _ = framed.send(Bytes::from(payload)).await;
                continue;
            }
        };

        tracing::info!(
            event = "request_received",
            id = %request.id,
            client = %request.client,
            peer = %addr,
            command = %request_summary(&request),
        );

        let received_at = SystemTime::now();
        let record = RequestRecord::from_request(&request, &addr.to_string(), received_at);
        spawn_write_request_record_value(Arc::clone(&output_dir), record);

        let response = execute_request(&request, &whitelist, &limits, &output_dir).await;
        let payload = serde_json::to_vec(&response)?;
        framed.send(Bytes::from(payload)).await?;
    }
    Ok(())
}

fn handle_key_event(
    key: KeyEvent,
    app: &mut AppState,
    ui_tx: mpsc::Sender<UiEvent>,
    whitelist: Arc<Whitelist>,
    limits: Arc<crate::config::LimitsConfig>,
    output_dir: Arc<PathBuf>,
) -> bool {
    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => return true,
        KeyCode::Down | KeyCode::Char('j') => app.select_next(),
        KeyCode::Up | KeyCode::Char('k') => app.select_prev(),
        KeyCode::Char('a') | KeyCode::Char('A') => {
            if let Some(pending) = app.pop_selected() {
                tracing::info!(
                    event = "request_approved",
                    id = %pending.request.id,
                    command = %request_summary(&pending.request),
                );
                let ui_tx = ui_tx.clone();
                let whitelist = Arc::clone(&whitelist);
                let limits = Arc::clone(&limits);
                let output_dir = Arc::clone(&output_dir);
                tokio::spawn(async move {
                    let response =
                        execute_request(&pending.request, &whitelist, &limits, &output_dir).await;
                    let record = ExecutionRecord::from_response(&pending, &response);
                    let _ = pending.respond_to.send(response);
                    let _ = ui_tx.send(UiEvent::Execution(record)).await;
                });
            }
        }
        KeyCode::Char('d') | KeyCode::Char('D') => {
            if let Some(pending) = app.pop_selected() {
                tracing::info!(
                    event = "request_denied",
                    id = %pending.request.id,
                    command = %request_summary(&pending.request),
                );
                let response =
                    CommandResponse::denied(pending.request.id.clone(), "denied by operator");
                let record = ExecutionRecord::from_response(&pending, &response);
                let _ = pending.respond_to.send(response.clone());
                let _ = ui_tx.try_send(UiEvent::Execution(record));
                let output_dir = Arc::clone(&output_dir);
                tokio::spawn(async move {
                    crate::executor::write_result_record(
                        &output_dir,
                        &response,
                        Duration::from_secs(0),
                    )
                    .await;
                });
            }
        }
        _ => {}
    }
    false
}

#[derive(Default)]
struct AppState {
    queue: Vec<PendingRequest>,
    selected: usize,
    list_state: ListState,
    connections: usize,
    last_result: Option<ExecutionRecord>,
}

impl AppState {
    fn handle_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::ConnectionOpened => self.connections += 1,
            UiEvent::ConnectionClosed => {
                self.connections = self.connections.saturating_sub(1);
            }
            UiEvent::Request(pending) => {
                self.queue.push(pending);
                if self.queue.len() == 1 {
                    self.selected = 0;
                }
            }
            UiEvent::Execution(record) => {
                self.last_result = Some(record);
            }
        }
        self.sync_selection();
    }

    fn select_next(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.queue.len();
        self.sync_selection();
    }

    fn select_prev(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        if self.selected == 0 {
            self.selected = self.queue.len() - 1;
        } else {
            self.selected -= 1;
        }
        self.sync_selection();
    }

    fn pop_selected(&mut self) -> Option<PendingRequest> {
        if self.queue.is_empty() {
            return None;
        }
        let index = self.selected.min(self.queue.len() - 1);
        let item = self.queue.remove(index);
        if self.selected >= self.queue.len() && !self.queue.is_empty() {
            self.selected = self.queue.len() - 1;
        }
        self.sync_selection();
        Some(item)
    }

    fn sync_selection(&mut self) {
        if self.queue.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(self.selected));
        }
    }
}

struct PendingRequest {
    request: CommandRequest,
    peer: String,
    received_at: SystemTime,
    queued_at: Instant,
    respond_to: oneshot::Sender<CommandResponse>,
}

struct ExecutionRecord {
    id: String,
    status: CommandStatus,
    exit_code: Option<i32>,
    summary: String,
    stdout: Option<String>,
    stderr: Option<String>,
}

impl ExecutionRecord {
    fn from_response(pending: &PendingRequest, response: &CommandResponse) -> Self {
        let summary = match response.status {
            CommandStatus::Completed => format!("completed (exit={:?})", response.exit_code),
            CommandStatus::Denied => "denied".to_string(),
            CommandStatus::Error => "error".to_string(),
            CommandStatus::Approved => "approved".to_string(),
        };
        Self {
            id: pending.request.id.clone(),
            status: response.status.clone(),
            exit_code: response.exit_code,
            summary,
            stdout: response.stdout.clone(),
            stderr: response.stderr.clone(),
        }
    }
}

enum UiEvent {
    ConnectionOpened,
    ConnectionClosed,
    Request(PendingRequest),
    Execution(ExecutionRecord),
}

#[derive(Serialize)]
struct RequestRecord {
    id: String,
    client: String,
    peer: String,
    received_at_ms: u64,
    intent: String,
    mode: protocol::CommandMode,
    command: String,
    raw_command: String,
    cwd: Option<String>,
    env: Option<std::collections::BTreeMap<String, String>>,
    timeout_ms: Option<u64>,
    max_output_bytes: Option<u64>,
    pipeline: Vec<CommandStage>,
}

impl RequestRecord {
    fn from_request(request: &CommandRequest, peer: &str, received_at: SystemTime) -> Self {
        Self {
            id: request.id.clone(),
            client: request.client.clone(),
            peer: peer.to_string(),
            received_at_ms: system_time_ms(received_at),
            intent: request.intent.clone(),
            mode: request.mode.clone(),
            command: request.raw_command.clone(),
            raw_command: request.raw_command.clone(),
            cwd: request.cwd.clone(),
            env: request.env.clone(),
            timeout_ms: request.timeout_ms,
            max_output_bytes: request.max_output_bytes,
            pipeline: request.pipeline.clone(),
        }
    }
}

fn spawn_write_request_record(output_dir: Arc<PathBuf>, pending: &PendingRequest) {
    let record = RequestRecord::from_request(&pending.request, &pending.peer, pending.received_at);
    spawn_write_request_record_value(output_dir, record);
}

fn spawn_write_request_record_value(output_dir: Arc<PathBuf>, record: RequestRecord) {
    tokio::spawn(async move {
        if let Err(err) = write_request_record(&output_dir, &record).await {
            tracing::warn!(error = %err, "failed to write request record");
        }
    });
}

fn draw_ui(frame: &mut ratatui::Frame, app: &mut AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(7),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[0]);

    let queue_items: Vec<ListItem> = app
        .queue
        .iter()
        .map(|pending| {
            let title = format!(
                "{}  {}",
                pending.request.id,
                request_summary(&pending.request)
            );
            ListItem::new(Line::from(title))
        })
        .collect();
    let queue = List::new(queue_items)
        .block(Block::default().title("Pending").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");
    frame.render_stateful_widget(queue, body[0], &mut app.list_state);

    let details = if let Some(selected) = app.queue.get(app.selected) {
        format_request_details(selected)
    } else {
        "no pending request".to_string()
    };
    let detail_block = Paragraph::new(details)
        .block(Block::default().title("Details").borders(Borders::ALL))
        .wrap(Wrap { trim: true });
    frame.render_widget(detail_block, body[1]);

    let result_text = if let Some(result) = &app.last_result {
        format_result_details(result)
    } else {
        "no execution yet".to_string()
    };
    let result_block = Paragraph::new(result_text)
        .block(Block::default().title("Last Result").borders(Borders::ALL))
        .wrap(Wrap { trim: true });
    frame.render_widget(result_block, chunks[1]);

    let footer = Paragraph::new(Line::from(vec![
        Span::raw("A=approve  D=deny  ↑/↓=select  Q=quit  "),
        Span::styled(
            format!("connections={}", app.connections),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(footer, chunks[2]);
}

fn format_request_details(pending: &PendingRequest) -> String {
    let request = &pending.request;
    let mut lines = vec![
        format!("id: {}", request.id),
        format!("client: {}", request.client),
        format!("peer: {}", pending.peer),
        format!("intent: {}", request.intent),
        format!("mode: {}", format_mode(&request.mode)),
        format!("command: {}", request.raw_command),
    ];
    if !request.pipeline.is_empty() {
        lines.push(format!("pipeline: {}", format_pipeline(&request.pipeline)));
    }
    if let Some(cwd) = &request.cwd {
        lines.push(format!("cwd: {cwd}"));
    }
    if let Some(timeout) = request.timeout_ms {
        lines.push(format!("timeout_ms: {timeout}"));
    }
    if let Some(max) = request.max_output_bytes {
        lines.push(format!("max_output_bytes: {max}"));
    }
    lines.push(format!(
        "queued_for: {}s",
        pending.queued_at.elapsed().as_secs()
    ));
    lines.join("\n")
}

fn format_result_details(result: &ExecutionRecord) -> String {
    let mut lines = vec![
        format!("id: {}", result.id),
        format!("status: {:?}", result.status),
        format!("summary: {}", result.summary),
    ];
    if let Some(code) = result.exit_code {
        lines.push(format!("exit_code: {code}"));
    }
    if let Some(stdout) = &result.stdout {
        lines.push(format!("stdout: {stdout}"));
    }
    if let Some(stderr) = &result.stderr {
        lines.push(format!("stderr: {stderr}"));
    }
    lines.join("\n")
}

fn deny_message(whitelist: &Whitelist, request: &CommandRequest) -> Option<String> {
    for stage in &request.pipeline {
        if let Err(message) = whitelist.validate_deny(stage) {
            return Some(message);
        }
    }
    None
}

fn request_summary(request: &CommandRequest) -> String {
    match &request.mode {
        CommandMode::Shell => request.raw_command.clone(),
        CommandMode::Argv => {
            let pipeline = format_pipeline(&request.pipeline);
            if pipeline.is_empty() {
                request.raw_command.clone()
            } else {
                pipeline
            }
        }
    }
}

fn format_mode(mode: &CommandMode) -> &'static str {
    match mode {
        CommandMode::Shell => "shell",
        CommandMode::Argv => "argv",
    }
}

fn format_pipeline(pipeline: &[CommandStage]) -> String {
    pipeline
        .iter()
        .map(|stage| stage.argv.join(" "))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn setup_terminal() -> anyhow::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> anyhow::Result<()> {
    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn init_tracing(
    audit_dir: &PathBuf,
    log_to_stderr: bool,
) -> anyhow::Result<tracing_appender::non_blocking::WorkerGuard> {
    std::fs::create_dir_all(audit_dir)?;
    let file_appender = tracing_appender::rolling::daily(audit_dir, "audit.log");
    let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_writer)
        .with_target(false)
        .json();

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let registry = tracing_subscriber::registry().with(filter).with(file_layer);

    if log_to_stderr {
        let stderr_layer = tracing_subscriber::fmt::layer()
            .with_writer(io::stderr)
            .with_target(false);
        registry.with(stderr_layer).init();
    } else {
        registry.init();
    }

    Ok(file_guard)
}

async fn write_request_record(output_dir: &Path, record: &RequestRecord) -> anyhow::Result<()> {
    let path = output_dir.join(format!("{}.request.json", record.id));
    let payload = serde_json::to_vec_pretty(record)?;
    tokio::fs::write(path, payload).await?;
    Ok(())
}

fn system_time_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
