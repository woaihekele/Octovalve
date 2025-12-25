pub(crate) mod app;
pub(crate) mod terminal;
pub(crate) mod theme;

use crate::layers::service::events::ServiceCommand;
use crate::shared::dto::{RequestView, ResultView};
use app::{ListView, ViewMode};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Clear, List, ListItem, Paragraph, Wrap};
use theme::{Theme, ValueStyle};
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
        .constraints([
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(chunks[1]);

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
        .style(theme.value_style(ValueStyle::Normal))
        .highlight_style(if app.list_view == ListView::Pending {
            theme.highlight_style()
        } else {
            theme.value_style(ValueStyle::Dim)
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
            theme.value_style(ValueStyle::Dim),
        ))]
    } else {
        let history_block = theme.block(history_title);
        let history_inner = history_block.inner(left[1]);
        let available_width = history_inner.width.saturating_sub(3) as usize;
        app.history
            .iter()
            .map(|result| {
                let exec_time = format_exec_time(result.finished_at_ms);
                let time_width = display_width(&exec_time);
                let gap = 2usize;
                if available_width <= time_width {
                    return ListItem::new(Line::styled(
                        exec_time,
                        theme.value_style(ValueStyle::Dim),
                    ));
                }
                let max_cmd = available_width.saturating_sub(time_width + gap);
                let command = truncate_with_ellipsis(&result.command, max_cmd);
                let padding_width = available_width.saturating_sub(time_width);
                let padded = pad_right(&command, padding_width);
                let line = Line::from(vec![
                    Span::styled(padded, theme.value_style(ValueStyle::Normal)),
                    Span::styled(exec_time, theme.value_style(ValueStyle::Dim)),
                ]);
                ListItem::new(line)
            })
            .collect::<Vec<_>>()
    };
    let history_list = List::new(history_items)
        .block(theme.block(history_title))
        .style(theme.value_style(ValueStyle::Normal))
        .highlight_style(if app.list_view == ListView::History {
            theme.highlight_style()
        } else {
            theme.value_style(ValueStyle::Dim)
        })
        .highlight_symbol(if app.list_view == ListView::History {
            ">> "
        } else {
            "   "
        });
    frame.render_stateful_widget(history_list, left[1], &mut app.history_list_state);

    let hostname = if app.hostname.is_empty() {
        "unknown"
    } else {
        app.hostname.as_str()
    };
    let host_ip = if app.host_ip.is_empty() {
        "unknown"
    } else {
        app.host_ip.as_str()
    };
    let header_line = Line::from(vec![
        Span::styled("Host: ", theme.key_style()),
        Span::styled(hostname, theme.value_style(ValueStyle::Important)),
        Span::styled("  IP: ", theme.key_style()),
        Span::styled(host_ip, theme.value_style(ValueStyle::Important)),
    ]);
    let header = Paragraph::new(header_line)
        .block(theme.block("Host"))
        .style(theme.value_style(ValueStyle::Normal));
    frame.render_widget(header, chunks[0]);

    let detail_title = match app.list_view {
        ListView::Pending => "Details",
        ListView::History => "Result Details",
    };
    let detail_block = theme.block(detail_title);
    let detail_inner = detail_block.inner(right[0]);
    let details = match app.list_view {
        ListView::Pending => app
            .queue
            .get(app.pending_selected)
            .map(|pending| format_request_details(&theme, pending, detail_inner.width))
            .unwrap_or_else(|| Text::from("no pending request")),
        ListView::History => app
            .selected_history()
            .map(|result| format_result_details(&theme, result, detail_inner.width))
            .unwrap_or_else(|| Text::from("no history result")),
    };
    let detail_widget = Paragraph::new(details)
        .block(detail_block)
        .style(theme.value_style(ValueStyle::Normal))
        .wrap(Wrap { trim: true });
    frame.render_widget(Clear, right[0]);
    frame.render_widget(detail_widget, right[0]);

    let result_title = match app.list_view {
        ListView::Pending => "Last Result",
        ListView::History => "Selected Output",
    };
    let result_block = theme.block(result_title);
    let result_inner = result_block.inner(right[1]);
    let result_widget = match app.list_view {
        ListView::Pending => Paragraph::new(
            app.last_result
                .as_ref()
                .map(|result| format_result_details(&theme, result, result_inner.width))
                .unwrap_or_else(|| Text::from("no execution yet")),
        ),
        ListView::History => {
            let output = app
                .selected_history()
                .map(format_result_output)
                .unwrap_or_else(|| "no output".to_string());
            let wrapped = wrap_text_lines(&output, result_inner.width.max(1) as usize);
            Paragraph::new(wrapped.join("\n"))
        }
    };
    let result_widget = result_widget
        .block(result_block)
        .style(theme.value_style(ValueStyle::Normal))
        .wrap(Wrap { trim: true });
    frame.render_widget(Clear, right[1]);
    frame.render_widget(result_widget, right[1]);

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
    let footer = Paragraph::new(Line::from(footer_spans)).block(theme.block("Controls"));
    frame.render_widget(footer, chunks[2]);
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
            .map(format_result_output)
            .unwrap_or_else(|| "no output".to_string()),
        ListView::Pending => app
            .last_result
            .as_ref()
            .map(format_result_output)
            .unwrap_or_else(|| "no output".to_string()),
    };

    let result_block = theme.block("Result (fullscreen)");
    let inner = result_block.inner(chunks[0]);
    let wrapped = wrap_text_lines(&result_text, inner.width.max(1) as usize);
    app.set_result_metrics(wrapped.len(), inner.height);
    let rendered = wrapped.join("\n");

    let result_panel = Paragraph::new(rendered)
        .block(result_block)
        .style(theme.value_style(ValueStyle::Normal))
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

const TAB_WIDTH: usize = 4;

fn sanitize_text_for_tui(text: &str) -> String {
    let stripped = strip_ansi_sequences(text);
    let mut out = String::with_capacity(stripped.len());
    let tab_width = TAB_WIDTH.max(1);
    let mut col = 0usize;
    for ch in stripped.chars() {
        match ch {
            '\n' => {
                out.push('\n');
                col = 0;
            }
            '\r' => {
                out.push('\n');
                col = 0;
            }
            '\t' => {
                let spaces = tab_width.saturating_sub(col % tab_width).max(1);
                out.extend(std::iter::repeat(' ').take(spaces));
                col += spaces;
            }
            _ if ch.is_control() => {
                out.push(' ');
                col += 1;
            }
            _ => {
                out.push(ch);
                col += 1;
            }
        }
    }
    out
}

fn strip_ansi_sequences(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    while let Some(seq_ch) = chars.next() {
                        if ('@'..='~').contains(&seq_ch) {
                            break;
                        }
                    }
                    continue;
                }
                Some(']') => {
                    chars.next();
                    loop {
                        match chars.next() {
                            Some('\u{7}') => break,
                            Some('\u{1b}') => {
                                if let Some('\\') = chars.peek().copied() {
                                    chars.next();
                                }
                                break;
                            }
                            Some(_) => continue,
                            None => break,
                        }
                    }
                    continue;
                }
                _ => continue,
            }
        }
        out.push(ch);
    }
    out
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

fn format_request_details(theme: &Theme, pending: &RequestView, width: u16) -> Text<'static> {
    let mut lines = Vec::new();
    lines.extend(kv_lines(
        theme,
        "id",
        pending.id.clone(),
        ValueStyle::Dim,
        width,
    ));
    lines.extend(kv_lines(
        theme,
        "client",
        pending.client.clone(),
        ValueStyle::Normal,
        width,
    ));
    lines.extend(kv_lines(
        theme,
        "target",
        pending.target.clone(),
        ValueStyle::Important,
        width,
    ));
    lines.extend(kv_lines(
        theme,
        "peer",
        pending.peer.clone(),
        ValueStyle::Normal,
        width,
    ));
    lines.extend(kv_lines(
        theme,
        "intent",
        pending.intent.clone(),
        ValueStyle::Important,
        width,
    ));
    lines.extend(kv_lines(
        theme,
        "mode",
        pending.mode.clone(),
        ValueStyle::Normal,
        width,
    ));
    lines.extend(kv_lines(
        theme,
        "command",
        pending.command.clone(),
        ValueStyle::Important,
        width,
    ));
    if let Some(pipeline) = &pending.pipeline {
        lines.extend(kv_lines(
            theme,
            "pipeline",
            pipeline.clone(),
            ValueStyle::Normal,
            width,
        ));
    }
    if let Some(cwd) = &pending.cwd {
        lines.extend(kv_lines(
            theme,
            "cwd",
            cwd.clone(),
            ValueStyle::Normal,
            width,
        ));
    }
    if let Some(timeout) = pending.timeout_ms {
        lines.extend(kv_lines(
            theme,
            "timeout_ms",
            timeout.to_string(),
            ValueStyle::Normal,
            width,
        ));
    }
    if let Some(max) = pending.max_output_bytes {
        lines.extend(kv_lines(
            theme,
            "max_output_bytes",
            max.to_string(),
            ValueStyle::Normal,
            width,
        ));
    }
    lines.extend(kv_lines(
        theme,
        "queued_for",
        format!("{}s", pending.queued_at.elapsed().as_secs()),
        ValueStyle::Dim,
        width,
    ));
    Text::from(lines)
}

fn format_result_details(theme: &Theme, result: &ResultView, width: u16) -> Text<'static> {
    let mut lines = Vec::new();
    lines.extend(kv_lines(
        theme,
        "id",
        result.id.clone(),
        ValueStyle::Dim,
        width,
    ));
    lines.extend(kv_lines(
        theme,
        "intent",
        result.intent.clone(),
        ValueStyle::Important,
        width,
    ));
    lines.extend(kv_lines(
        theme,
        "cwd",
        result
            .cwd
            .clone()
            .unwrap_or_else(|| "(default)".to_string()),
        ValueStyle::Important,
        width,
    ));
    lines.extend(kv_lines(
        theme,
        "command",
        result.command.clone(),
        ValueStyle::Important,
        width,
    ));
    lines.extend(kv_lines(
        theme,
        "mode",
        result.mode.clone(),
        ValueStyle::Normal,
        width,
    ));
    lines.extend(kv_lines(
        theme,
        "summary",
        result.summary.clone(),
        ValueStyle::Important,
        width,
    ));
    lines.extend(kv_lines(
        theme,
        "peer",
        result.peer.clone(),
        ValueStyle::Normal,
        width,
    ));
    lines.extend(kv_lines(
        theme,
        "pipeline",
        result.pipeline.clone().unwrap_or_else(|| "-".to_string()),
        ValueStyle::Normal,
        width,
    ));
    lines.extend(kv_lines(
        theme,
        "queued_for",
        format!("{}s", result.queued_for_secs),
        ValueStyle::Dim,
        width,
    ));
    Text::from(lines)
}

fn format_result_output(result: &ResultView) -> String {
    let mut lines = Vec::new();
    if let Some(stdout) = &result.stdout {
        let cleaned = sanitize_text_for_tui(stdout);
        lines.push(format!("stdout: {cleaned}"));
    }
    if let Some(stderr) = &result.stderr {
        let cleaned = sanitize_text_for_tui(stderr);
        lines.push(format!("stderr: {cleaned}"));
    }
    if lines.is_empty() {
        "no output".to_string()
    } else {
        lines.join("\n")
    }
}

fn kv_lines(
    theme: &Theme,
    key: &str,
    value: String,
    level: ValueStyle,
    width: u16,
) -> Vec<Line<'static>> {
    let value = sanitize_text_for_tui(&value);
    let key_label = format!("{key}: ");
    let key_width = display_width(&key_label);
    let width = width.max(1) as usize;
    let value_width = width.saturating_sub(key_width).max(1);
    let wrapped = wrap_text_lines(&value, value_width);
    let mut lines = Vec::with_capacity(wrapped.len().max(1));
    let indent = " ".repeat(key_width);
    for (idx, segment) in wrapped.into_iter().enumerate() {
        if idx == 0 {
            lines.push(Line::from(vec![
                Span::styled(key_label.clone(), theme.key_style()),
                Span::styled(segment, theme.value_style(level)),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(indent.clone(), theme.key_style()),
                Span::styled(segment, theme.value_style(level)),
            ]));
        }
    }
    lines
}

fn display_width(text: &str) -> usize {
    text.chars().count()
}

fn pad_right(text: &str, width: usize) -> String {
    let mut out = text.to_string();
    let current = display_width(text);
    if current < width {
        out.extend(std::iter::repeat(' ').take(width - current));
    }
    out
}

fn truncate_with_ellipsis(text: &str, max_len: usize) -> String {
    if max_len == 0 {
        return String::new();
    }
    let chars = text.chars();
    let count = chars.clone().count();
    if count <= max_len {
        return text.to_string();
    }
    if max_len <= 3 {
        return chars.take(max_len).collect();
    }
    let keep = max_len - 3;
    let mut out: String = chars.take(keep).collect();
    out.push_str("...");
    out
}

fn format_exec_time(finished_at_ms: u64) -> String {
    let secs = finished_at_ms / 1000;
    let secs_in_day = secs % 86_400;
    let hours = secs_in_day / 3_600;
    let minutes = (secs_in_day % 3_600) / 60;
    let seconds = secs_in_day % 60;
    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}
