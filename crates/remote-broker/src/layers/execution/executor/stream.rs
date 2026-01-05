use std::io;
use std::sync::Arc;

use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

pub(super) async fn read_stream_capture<R: AsyncRead + Unpin>(
    mut reader: R,
    max_bytes: usize,
    writer: Option<Arc<Mutex<File>>>,
) -> io::Result<(Vec<u8>, bool)> {
    let mut buffer = Vec::new();
    let mut truncated = false;
    let mut chunk = [0u8; 4096];
    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        if let Some(writer) = &writer {
            write_chunk(writer, &chunk[..n]).await?;
        }
        if buffer.len() < max_bytes {
            let remaining = max_bytes - buffer.len();
            let to_copy = remaining.min(n);
            buffer.extend_from_slice(&chunk[..to_copy]);
            if to_copy < n {
                truncated = true;
            }
        } else {
            truncated = true;
        }
    }
    Ok((buffer, truncated))
}

async fn write_chunk(writer: &Arc<Mutex<File>>, data: &[u8]) -> io::Result<()> {
    let mut file = writer.lock().await;
    file.write_all(data).await
}
