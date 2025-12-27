mod activity;
mod cli;
mod layers;

use crate::activity::{spawn_idle_shutdown, ActivityTracker};
use crate::cli::Args;
use crate::layers::policy::config::Config;
use crate::layers::policy::whitelist::Whitelist;
use crate::layers::service::events::{ServiceCommand, ServiceEvent};
use crate::layers::service::history::load_history;
use crate::layers::service::logging::init_tracing;
use crate::layers::service::{run_headless, run_tui_service, spawn_control_server};
use crate::layers::ui::app::HISTORY_LIMIT;
use crate::layers::ui::{draw_ui, handle_key_event, restore_terminal, setup_terminal, AppState};
use anyhow::Context;
use clap::Parser;
use crossterm::event::{self, Event};
use std::net::UdpSocket;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::broadcast::error::TryRecvError;
use tokio::sync::{broadcast, mpsc};

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
    let activity = Arc::new(ActivityTracker::new());

    let listener = TcpListener::bind(&args.listen_addr)
        .await
        .with_context(|| format!("failed to bind {}", args.listen_addr))?;

    if args.auto_approve {
        if args.control_addr.is_some() {
            tracing::warn!("control api is disabled in auto-approve mode");
        }
        if args.idle_exit_secs > 0 {
            spawn_idle_shutdown(activity.clone(), Duration::from_secs(args.idle_exit_secs));
        }
        run_headless(listener, whitelist, limits, output_dir, activity).await?;
        return Ok(());
    }

    let (event_tx, _) = broadcast::channel::<ServiceEvent>(128);
    let (cmd_tx, cmd_rx) = mpsc::channel::<ServiceCommand>(128);

    let history = load_history(output_dir.as_ref(), limits.max_output_bytes, HISTORY_LIMIT);

    run_tui_service(
        listener,
        whitelist,
        Arc::clone(&limits),
        Arc::clone(&output_dir),
        config.auto_approve_allowed,
        history.clone(),
        HISTORY_LIMIT,
        event_tx.clone(),
        cmd_rx,
        activity.clone(),
    );

    if let Some(control_addr) = args.control_addr.clone() {
        spawn_control_server(
            control_addr,
            cmd_tx.clone(),
            event_tx.clone(),
            activity.clone(),
        )
        .await?;
    } else if args.headless {
        tracing::warn!("headless mode without control api will require local approvals");
    }

    if args.headless {
        if args.idle_exit_secs > 0 {
            spawn_idle_shutdown(activity, Duration::from_secs(args.idle_exit_secs));
        }
        tokio::signal::ctrl_c().await?;
        return Ok(());
    }

    let mut terminal = setup_terminal()?;
    let mut app = AppState::default();
    app.set_host_info(resolve_hostname(), resolve_ip());
    app.load_history(history);

    let tick_rate = Duration::from_millis(100);
    let mut ui_event_rx = event_tx.subscribe();
    loop {
        loop {
            match ui_event_rx.try_recv() {
                Ok(event) => app.handle_event(event),
                Err(TryRecvError::Lagged(_)) => continue,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Closed) => break,
            }
        }

        terminal.draw(|frame| draw_ui(frame, &mut app))?;

        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                if handle_key_event(key, &mut app, cmd_tx.clone()) {
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
