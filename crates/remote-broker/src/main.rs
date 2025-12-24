mod app;
mod audit;
mod cli;
mod config;
mod executor;
mod format;
mod server;
mod terminal;
mod ui;
mod whitelist;

use crate::app::{AppState, UiEvent};
use crate::cli::Args;
use crate::config::Config;
use crate::server::{run_headless, spawn_accept_loop};
use crate::terminal::{init_tracing, restore_terminal, setup_terminal};
use crate::ui::{draw_ui, handle_key_event};
use crate::whitelist::Whitelist;
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
