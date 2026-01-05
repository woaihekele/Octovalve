pub(super) fn wrap_text_lines(text: &str, width: usize) -> Vec<String> {
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

pub(super) fn sanitize_text_for_tui(text: &str) -> String {
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

pub(super) fn display_width(text: &str) -> usize {
    text.chars().count()
}

pub(super) fn pad_right(text: &str, width: usize) -> String {
    let mut out = text.to_string();
    let current = display_width(text);
    if current < width {
        out.extend(std::iter::repeat(' ').take(width - current));
    }
    out
}

pub(super) fn truncate_with_ellipsis(text: &str, max_len: usize) -> String {
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
