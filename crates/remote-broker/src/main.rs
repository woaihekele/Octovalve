mod cli;
mod layers;
mod shared;

use crate::cli::Args;
use crate::layers::policy::config::Config;
use crate::layers::policy::whitelist::Whitelist;
use crate::layers::service::events::{ServiceCommand, ServiceEvent};
use crate::layers::service::history::load_history;
use crate::layers::service::logging::init_tracing;
use crate::layers::service::{run_headless, run_tui_service};
use crate::layers::ui::app::HISTORY_LIMIT;
use crate::layers::ui::{draw_ui, handle_key_event, restore_terminal, setup_terminal, AppState};
use anyhow::Context;
use clap::Parser;
use crossterm::event::{self, Event};
use std::net::UdpSocket;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

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

    let (ui_event_tx, mut ui_event_rx) = mpsc::channel::<ServiceEvent>(128);
    let (ui_cmd_tx, ui_cmd_rx) = mpsc::channel::<ServiceCommand>(128);

    run_tui_service(
        listener,
        whitelist,
        limits,
        output_dir,
        config.auto_approve_allowed,
        ui_event_tx,
        ui_cmd_rx,
    );

    let mut terminal = setup_terminal()?;
    let mut app = AppState::default();
    app.set_host_info(resolve_hostname(), resolve_ip());
    let history = load_history(output_dir.as_ref(), limits.max_output_bytes, HISTORY_LIMIT);
    app.load_history(history);

    let tick_rate = Duration::from_millis(100);
    loop {
        while let Ok(event) = ui_event_rx.try_recv() {
            app.handle_event(event);
        }

        terminal.draw(|frame| draw_ui(frame, &mut app))?;

        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                if handle_key_event(key, &mut app, ui_cmd_tx.clone()) {
                    break;
                }
            }
        }
    }

    restore_terminal(&mut terminal)?;
    Ok(())
}

fn resolve_hostname() -> String {
    if let Ok(value) = std::env::var("HOSTNAME") {
        if !value.trim().is_empty() {
            return value;
        }
    }
    let mut buf = [0u8; 256];
    let result = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut i8, buf.len()) };
    if result == 0 {
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..len]).to_string()
    } else {
        "unknown".to_string()
    }
}

fn resolve_ip() -> String {
    let socket = UdpSocket::bind("0.0.0.0:0");
    if let Ok(sock) = socket {
        if sock.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = sock.local_addr() {
                return addr.ip().to_string();
            }
        }
    }
    "unknown".to_string()
}
