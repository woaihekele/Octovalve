mod cli;
mod layers;
mod shared;

use crate::cli::Args;
use crate::layers::policy::config::Config;
use crate::layers::policy::whitelist::Whitelist;
use crate::layers::service::events::{ServiceCommand, ServiceEvent};
use crate::layers::service::{run_headless, run_tui_service};
use crate::layers::service::logging::init_tracing;
use crate::layers::ui::{draw_ui, handle_key_event, AppState, restore_terminal, setup_terminal};
use anyhow::Context;
use clap::Parser;
use crossterm::event::{self, Event};
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

    run_tui_service(listener, whitelist, limits, output_dir, ui_event_tx, ui_cmd_rx);

    let mut terminal = setup_terminal()?;
    let mut app = AppState::default();

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
