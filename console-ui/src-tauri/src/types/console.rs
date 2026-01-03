use serde::Serialize;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogChunk {
    pub content: String,
    pub next_offset: u64,
}
