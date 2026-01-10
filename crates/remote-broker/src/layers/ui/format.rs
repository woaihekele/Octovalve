use crate::layers::policy::summary::{format_mode, format_pipeline};
use protocol::control::{RequestSnapshot, ResultSnapshot};
use protocol::{CommandMode, CommandStatus};
use ratatui::text::{Line, Span, Text};
use std::time::{SystemTime, UNIX_EPOCH};

use super::text::{display_width, sanitize_text_for_tui, wrap_text_lines};
use super::theme::{Theme, ValueStyle};

pub(super) fn format_request_details(
    theme: &Theme,
    pending: &RequestSnapshot,
    width: u16,
) -> Text<'static> {
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
        format_mode(&pending.mode).to_string(),
        ValueStyle::Normal,
        width,
    ));
    lines.extend(kv_lines(
        theme,
        "command",
        pending.raw_command.clone(),
        ValueStyle::Important,
        width,
    ));
    if !pending.pipeline.is_empty() {
        lines.extend(kv_lines(
            theme,
            "pipeline",
            format_pipeline(&pending.pipeline),
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
        format!("{}s", queued_for_secs(pending.received_at_ms)),
        ValueStyle::Dim,
        width,
    ));
    Text::from(lines)
}

pub(super) fn format_result_details(
    theme: &Theme,
    result: &ResultSnapshot,
    width: u16,
) -> Text<'static> {
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
        result.raw_command.clone(),
        ValueStyle::Important,
        width,
    ));
    lines.extend(kv_lines(
        theme,
        "mode",
        format_mode(&result.mode).to_string(),
        ValueStyle::Normal,
        width,
    ));
    lines.extend(kv_lines(
        theme,
        "summary",
        format_result_summary(result),
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
        format_pipeline_or_command(&result.raw_command, &result.pipeline),
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

pub(super) fn format_result_output(result: &ResultSnapshot) -> String {
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

pub(super) fn request_summary(pending: &RequestSnapshot) -> String {
    let pipeline = format_pipeline(&pending.pipeline);
    if pipeline.is_empty() {
        pending.raw_command.clone()
    } else {
        pipeline
    }
}

pub(super) fn format_result_summary(result: &ResultSnapshot) -> String {
    match result.status {
        CommandStatus::Completed => format!("completed (exit={:?})", result.exit_code),
        CommandStatus::Denied => "denied".to_string(),
        CommandStatus::Error => "error".to_string(),
        CommandStatus::Approved => "approved".to_string(),
        CommandStatus::Cancelled => "cancelled".to_string(),
    }
}

fn format_pipeline_or_command(command: &str, pipeline: &[protocol::CommandStage]) -> String {
    if pipeline.is_empty() {
        command.to_string()
    } else {
        format_pipeline(pipeline)
    }
}

fn queued_for_secs(received_at_ms: u64) -> u64 {
    let now_ms = system_time_ms(SystemTime::now());
    if received_at_ms == 0 {
        0
    } else {
        now_ms.saturating_sub(received_at_ms) / 1000
    }
}

fn system_time_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
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

pub(super) fn format_exec_time(finished_at_ms: u64) -> String {
    let secs = finished_at_ms / 1000;
    let secs_in_day = secs % 86_400;
    let hours = secs_in_day / 3_600;
    let minutes = (secs_in_day % 3_600) / 60;
    let seconds = secs_in_day % 60;
    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}
