use crate::layers::service::events::ServiceCommand;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;

use super::app::{AppState, ListView, ViewMode};

pub(crate) fn handle_key_event(
    key: KeyEvent,
    app: &mut AppState,
    cmd_tx: mpsc::Sender<ServiceCommand>,
) -> bool {
    if app.confirm_quit {
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => return true,
            KeyCode::Esc => {
                app.confirm_quit = false;
                return false;
            }
            _ => {
                app.confirm_quit = false;
            }
        }
    }

    if app.view_mode == ViewMode::ResultFullscreen {
        return handle_result_fullscreen_key(key, app);
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => app.confirm_quit = true,
        KeyCode::Down | KeyCode::Char('j') => app.select_next(),
        KeyCode::Up | KeyCode::Char('k') => app.select_prev(),
        KeyCode::Char('r') | KeyCode::Char('R') => app.enter_result_fullscreen(),
        KeyCode::Tab => {
            let next = if app.list_view == ListView::Pending {
                ListView::History
            } else {
                ListView::Pending
            };
            app.set_list_view(next);
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            if app.list_view == ListView::Pending {
                if let Some(id) = app.selected_request_id() {
                    let _ = cmd_tx.try_send(ServiceCommand::Approve(id));
                }
            }
        }
        KeyCode::Char('d') | KeyCode::Char('D') => {
            if app.list_view == ListView::Pending {
                if let Some(id) = app.selected_request_id() {
                    let _ = cmd_tx.try_send(ServiceCommand::Deny(id));
                }
            }
        }
        _ => {}
    }
    false
}

fn handle_result_fullscreen_key(key: KeyEvent, app: &mut AppState) -> bool {
    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => app.confirm_quit = true,
        KeyCode::Esc | KeyCode::Char('r') | KeyCode::Char('R') => app.exit_result_fullscreen(),
        KeyCode::Down | KeyCode::Char('j') => app.scroll_down(1),
        KeyCode::Up | KeyCode::Char('k') => app.scroll_up(1),
        KeyCode::PageDown => app.scroll_down(app.page_size()),
        KeyCode::PageUp => app.scroll_up(app.page_size()),
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.scroll_down(app.page_size());
        }
        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.scroll_up(app.page_size());
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.scroll_down(app.half_page_size());
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.scroll_up(app.half_page_size());
        }
        KeyCode::Char('g') => {
            if app.pending_g {
                app.scroll_to_top();
            } else {
                app.pending_g = true;
            }
        }
        KeyCode::Char('G') => app.scroll_to_bottom(),
        _ => app.pending_g = false,
    }
    false
}
