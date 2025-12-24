pub(crate) mod app;
pub(crate) mod terminal;

use crate::layers::service::events::ServiceCommand;
use crate::shared::dto::{RequestView, ResultView};
use app::{ListView, ViewMode};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use tokio::sync::mpsc;

pub(crate) use app::AppState;
pub(crate) use terminal::{restore_terminal, setup_terminal};

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

pub(crate) fn draw_ui(frame: &mut ratatui::Frame, app: &mut AppState) {
    if app.view_mode == ViewMode::ResultFullscreen {
        draw_result_fullscreen(frame, app);
        return;
    }

    let theme = Theme::dark();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(3)])
        .split(frame.area());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(chunks[0]);

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(body[0]);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(body[1]);

    let pending_title = if app.list_view == ListView::Pending {
        "Pending *"
    } else {
        "Pending"
    };
    let pending_items = app
        .queue
        .iter()
        .map(|pending| {
            let title = format!("{}  {}", pending.id, pending.summary);
            ListItem::new(Line::from(title))
        })
        .collect::<Vec<_>>();
    let pending_list = List::new(pending_items)
        .block(theme.block(pending_title))
        .style(Style::default().fg(theme.text))
        .highlight_style(if app.list_view == ListView::Pending {
            theme.highlight_style()
        } else {
            Style::default().fg(theme.dim)
        })
        .highlight_symbol(if app.list_view == ListView::Pending {
            ">> "
        } else {
            "   "
        });
    frame.render_stateful_widget(pending_list, left[0], &mut app.pending_list_state);

    let history_title = if app.list_view == ListView::History {
        "History (last 50) *"
    } else {
        "History (last 50)"
    };
    let history_items = if app.history.is_empty() {
        vec![ListItem::new(Line::styled(
            "no history yet",
            Style::default().fg(theme.dim),
        ))]
    } else {
        app.history
            .iter()
            .map(|result| {
                let title = format!("{}  {}", result.id, result.command);
                ListItem::new(Line::from(title))
            })
            .collect::<Vec<_>>()
    };
    let history_list = List::new(history_items)
        .block(theme.block(history_title))
        .style(Style::default().fg(theme.text))
        .highlight_style(if app.list_view == ListView::History {
            theme.highlight_style()
        } else {
            Style::default().fg(theme.dim)
        })
        .highlight_symbol(if app.list_view == ListView::History {
            ">> "
        } else {
            "   "
        });
    frame.render_stateful_widget(history_list, left[1], &mut app.history_list_state);

    let details = match app.list_view {
        ListView::Pending => app
            .queue
            .get(app.pending_selected)
            .map(format_request_details)
            .unwrap_or_else(|| "no pending request".to_string()),
        ListView::History => app
            .selected_history()
            .map(format_result_details)
            .unwrap_or_else(|| "no history result".to_string()),
    };
    let detail_title = match app.list_view {
        ListView::Pending => "Details",
        ListView::History => "Result Details",
    };
    let detail_block = Paragraph::new(details)
        .block(theme.block(detail_title))
        .style(Style::default().fg(theme.text))
        .wrap(Wrap { trim: true });
    frame.render_widget(detail_block, right[0]);

    let (result_title, result_text) = match app.list_view {
        ListView::Pending => (
            "Last Result",
            app.last_result
                .as_ref()
                .map(format_result_details)
                .unwrap_or_else(|| "no execution yet".to_string()),
        ),
        ListView::History => (
            "Selected Output",
            app.selected_history()
                .map(format_result_output)
                .unwrap_or_else(|| "no output".to_string()),
        ),
    };
    let result_block = Paragraph::new(result_text)
        .block(theme.block(result_title))
        .style(Style::default().fg(theme.text))
        .wrap(Wrap { trim: true });
    frame.render_widget(result_block, right[1]);

    let mut footer_spans = vec![Span::styled(
        "A=approve  D=deny  ↑/↓=select  Tab=focus  R=full  Q=quit  ",
        theme.help_style(),
    )];
    if app.confirm_quit {
        footer_spans.push(Span::styled(
            "再次按 Q 退出 / Esc 取消  ",
            theme.warn_style(),
        ));
    }
    footer_spans.push(Span::styled(
        format!("connections={}", app.connections),
        theme.accent_style(),
    ));
    let footer = Paragraph::new(Line::from(footer_spans)).block(theme.block("Controls"));
    frame.render_widget(footer, chunks[1]);
}

fn draw_result_fullscreen(frame: &mut ratatui::Frame, app: &mut AppState) {
    let theme = Theme::dark();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(frame.area());

    let result_text = match app.list_view {
        ListView::History => app
            .selected_history()
            .map(format_result_details)
            .unwrap_or_else(|| "no execution yet".to_string()),
        ListView::Pending => app
            .last_result
            .as_ref()
            .map(format_result_details)
            .unwrap_or_else(|| "no execution yet".to_string()),
    };

    let result_block = theme.block("Result (fullscreen)");
    let inner = result_block.inner(chunks[0]);
    let wrapped = wrap_text_lines(&result_text, inner.width.max(1) as usize);
    app.set_result_metrics(wrapped.len(), inner.height);
    let rendered = wrapped.join("\n");

    let result_panel = Paragraph::new(rendered)
        .block(result_block)
        .style(Style::default().fg(theme.text))
        .scroll((app.result_scroll as u16, 0));
    frame.render_widget(result_panel, chunks[0]);

    let mut footer_spans = vec![Span::styled(
        "j/k=scroll  gg/G=top/bottom  Ctrl+f/b=page  R/Esc=back  Q=quit  ",
        theme.help_style(),
    )];
    if app.confirm_quit {
        footer_spans.push(Span::styled(
            "再次按 Q 退出 / Esc 取消  ",
            theme.warn_style(),
        ));
    }
    footer_spans.push(Span::styled(
        format!(
            "line {}/{}",
            app.result_scroll.saturating_add(1),
            app.result_total_lines
        ),
        theme.accent_style(),
    ));
    let footer = Paragraph::new(Line::from(footer_spans)).block(theme.block("Controls"));
    frame.render_widget(footer, chunks[1]);
}

fn wrap_text_lines(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for raw in text.split('\n') {
        if raw.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut buffer = String::new();
        let mut count = 0usize;
        for ch in raw.chars() {
            buffer.push(ch);
            count += 1;
            if count >= width {
                lines.push(std::mem::take(&mut buffer));
                count = 0;
            }
        }
        if !buffer.is_empty() {
            lines.push(buffer);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
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

struct Theme {
    border: Color,
    title: Color,
    text: Color,
    dim: Color,
    accent: Color,
    highlight_fg: Color,
    highlight_bg: Color,
    warn: Color,
}

impl Theme {
    fn dark() -> Self {
        Self {
            border: Color::DarkGray,
            title: Color::Blue,
            text: Color::White,
            dim: Color::Gray,
            accent: Color::Cyan,
            highlight_fg: Color::White,
            highlight_bg: Color::DarkGray,
            warn: Color::Yellow,
        }
    }

    fn block<'a>(&self, title: &'a str) -> Block<'a> {
        Block::default()
            .title(Span::styled(
                title,
                Style::default()
                    .fg(self.title)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.border))
    }

    fn highlight_style(&self) -> Style {
        Style::default()
            .fg(self.highlight_fg)
            .bg(self.highlight_bg)
            .add_modifier(Modifier::BOLD)
    }

    fn help_style(&self) -> Style {
        Style::default().fg(self.dim)
    }

    fn accent_style(&self) -> Style {
        Style::default().fg(self.accent).add_modifier(Modifier::BOLD)
    }

    fn warn_style(&self) -> Style {
        Style::default().fg(self.warn).add_modifier(Modifier::BOLD)
    }
}

fn format_request_details(pending: &RequestView) -> String {
    let mut lines = vec![
        format!("id: {}", pending.id),
        format!("client: {}", pending.client),
        format!("target: {}", pending.target),
        format!("peer: {}", pending.peer),
        format!("intent: {}", pending.intent),
        format!("mode: {}", pending.mode),
        format!("command: {}", pending.command),
    ];
    if let Some(pipeline) = &pending.pipeline {
        lines.push(format!("pipeline: {pipeline}"));
    }
    if let Some(cwd) = &pending.cwd {
        lines.push(format!("cwd: {cwd}"));
    }
    if let Some(timeout) = pending.timeout_ms {
        lines.push(format!("timeout_ms: {timeout}"));
    }
    if let Some(max) = pending.max_output_bytes {
        lines.push(format!("max_output_bytes: {max}"));
    }
    lines.push(format!(
        "queued_for: {}s",
        pending.queued_at.elapsed().as_secs()
    ));
    lines.join("\n")
}

fn format_result_details(result: &ResultView) -> String {
    let mut lines = vec![
        format!("id: {}", result.id),
        format!("status: {}", result.status),
        format!("summary: {}", result.summary),
        format!("command: {}", result.command),
        format!("target: {}", result.target),
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

fn format_result_output(result: &ResultView) -> String {
    let mut lines = Vec::new();
    if let Some(stdout) = &result.stdout {
        lines.push(format!("stdout: {stdout}"));
    }
    if let Some(stderr) = &result.stderr {
        lines.push(format!("stderr: {stderr}"));
    }
    if lines.is_empty() {
        "no output".to_string()
    } else {
        lines.join("\n")
    }
}
