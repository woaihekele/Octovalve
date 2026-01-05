use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Clear, List, ListItem, Paragraph, Wrap};

use super::app::{AppState, ListView, ViewMode};
use super::format::{
    format_exec_time, format_request_details, format_result_details, format_result_output,
    request_summary,
};
use super::text::{display_width, pad_right, truncate_with_ellipsis, wrap_text_lines};
use super::theme::{Theme, ValueStyle};

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
            let title = format!("{}  {}", pending.id, request_summary(pending));
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
                let command = truncate_with_ellipsis(&result.raw_command, max_cmd);
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
