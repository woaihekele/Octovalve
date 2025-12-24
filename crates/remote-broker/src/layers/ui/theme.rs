use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy)]
pub(crate) enum ValueStyle {
    Normal,
    Important,
    Dim,
}

pub(crate) struct Theme {
    border: Color,
    title: Color,
    text: Color,
    dim: Color,
    accent: Color,
    highlight_fg: Color,
    highlight_bg: Color,
    warn: Color,
    ok: Color,
    error: Color,
}

impl Theme {
    pub(crate) fn dark() -> Self {
        Self {
            border: Color::DarkGray,
            title: Color::Blue,
            text: Color::White,
            dim: Color::Gray,
            accent: Color::Cyan,
            highlight_fg: Color::White,
            highlight_bg: Color::DarkGray,
            warn: Color::Yellow,
            ok: Color::Green,
            error: Color::Red,
        }
    }

    pub(crate) fn block<'a>(&self, title: &'a str) -> ratatui::widgets::Block<'a> {
        ratatui::widgets::Block::default()
            .title(ratatui::text::Span::styled(
                title,
                Style::default()
                    .fg(self.title)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(ratatui::widgets::Borders::ALL)
            .border_style(Style::default().fg(self.border))
    }

    pub(crate) fn highlight_style(&self) -> Style {
        Style::default()
            .fg(self.highlight_fg)
            .bg(self.highlight_bg)
            .add_modifier(Modifier::BOLD)
    }

    pub(crate) fn help_style(&self) -> Style {
        Style::default().fg(self.dim)
    }

    pub(crate) fn accent_style(&self) -> Style {
        Style::default().fg(self.accent).add_modifier(Modifier::BOLD)
    }

    pub(crate) fn warn_style(&self) -> Style {
        Style::default().fg(self.warn).add_modifier(Modifier::BOLD)
    }

    pub(crate) fn key_style(&self) -> Style {
        Style::default().fg(self.dim)
    }

    pub(crate) fn value_style(&self, level: ValueStyle) -> Style {
        match level {
            ValueStyle::Normal => Style::default().fg(self.text),
            ValueStyle::Important => Style::default()
                .fg(self.accent)
                .add_modifier(Modifier::BOLD),
            ValueStyle::Dim => Style::default().fg(self.dim),
        }
    }

    pub(crate) fn status_style(&self, status: &str) -> Style {
        match status {
            "Completed" => Style::default()
                .fg(self.ok)
                .add_modifier(Modifier::BOLD),
            "Denied" => Style::default()
                .fg(self.warn)
                .add_modifier(Modifier::BOLD),
            "Error" => Style::default()
                .fg(self.error)
                .add_modifier(Modifier::BOLD),
            "Approved" => Style::default()
                .fg(self.accent)
                .add_modifier(Modifier::BOLD),
            _ => Style::default().fg(self.text),
        }
    }
}
