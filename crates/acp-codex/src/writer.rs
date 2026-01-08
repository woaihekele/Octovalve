use anyhow::Result;
use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

pub(crate) struct AcpWriter {
    stdout: Mutex<Box<dyn tokio::io::AsyncWrite + Send + Unpin>>,
}

impl AcpWriter {
    pub(crate) fn new(writer: Box<dyn tokio::io::AsyncWrite + Send + Unpin>) -> Self {
        Self {
            stdout: Mutex::new(writer),
        }
    }

    pub(crate) fn stdio() -> Self {
        Self::new(Box::new(tokio::io::stdout()))
    }

    pub(crate) async fn send_json<T: Serialize + Sync>(&self, value: &T) -> Result<()> {
        let raw = serde_json::to_string(value)?;
        let mut guard = self.stdout.lock().await;
        guard.write_all(raw.as_bytes()).await?;
        guard.write_all(b"\n").await?;
        guard.flush().await?;
        Ok(())
    }
}
