use std::time::{Duration, Instant};

use anyhow::Context;
use bytes::Bytes;
use futures_util::SinkExt;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::control::ControlRequest;

const CONTROL_READY_TIMEOUT: Duration = Duration::from_secs(6);
const CONTROL_READY_INTERVAL: Duration = Duration::from_millis(200);
const CONTROL_READY_CONNECT_TIMEOUT: Duration = Duration::from_millis(500);

pub(crate) async fn wait_for_control_ready(
    target: &str,
    addr: &str,
    shutdown: &CancellationToken,
) -> anyhow::Result<()> {
    let start = Instant::now();
    let mut logged = false;
    loop {
        if shutdown.is_cancelled() {
            anyhow::bail!("shutdown requested");
        }
        match timeout(CONTROL_READY_CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => {
                drop(stream);
                return Ok(());
            }
            Ok(Err(_)) | Err(_) => {
                if !logged {
                    info!(
                        event = "control.ready.wait",
                        target = %target,
                        addr = %addr,
                        timeout_ms = CONTROL_READY_TIMEOUT.as_millis(),
                        "waiting for control listener"
                    );
                    logged = true;
                }
            }
        }
        if start.elapsed() >= CONTROL_READY_TIMEOUT {
            anyhow::bail!(
                "control addr not ready after {}ms",
                CONTROL_READY_TIMEOUT.as_millis()
            );
        }
        tokio::time::sleep(CONTROL_READY_INTERVAL).await;
    }
}

pub(crate) async fn connect_control(
    addr: &str,
) -> anyhow::Result<Framed<TcpStream, LengthDelimitedCodec>> {
    let stream = TcpStream::connect(addr)
        .await
        .with_context(|| format!("failed to connect control addr {addr}"))?;
    Ok(Framed::new(stream, LengthDelimitedCodec::new()))
}

pub(crate) async fn send_request(
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    request: ControlRequest,
) -> anyhow::Result<()> {
    let payload = serde_json::to_vec(&request)?;
    framed.send(Bytes::from(payload)).await?;
    Ok(())
}
