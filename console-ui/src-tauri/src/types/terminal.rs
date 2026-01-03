use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TerminalMessage {
    Ready { cols: u16, rows: u16, term: String },
    Output { data: String },
    Exit { code: Option<i32> },
    Error { message: String },
}
