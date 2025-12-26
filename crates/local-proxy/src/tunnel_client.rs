use anyhow::Context;
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_util::codec::{Framed, LinesCodec};
use tunnel_protocol::{ForwardSpec, ForwardStatus, TunnelRequest, TunnelResponse};

const TUNNEL_DAEMON_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(crate) struct TunnelClient {
    addr: String,
    client_id: String,
}

impl TunnelClient {
    pub(crate) fn new(addr: String, client_id: String) -> Self {
        Self { addr, client_id }
    }

    pub(crate) async fn ensure_forward(&self, forward: ForwardSpec) -> anyhow::Result<String> {
        let request = TunnelRequest::EnsureForward {
            client_id: self.client_id.clone(),
            forward,
        };
        match self.send_request(request).await? {
            TunnelResponse::EnsureForward { local_addr, .. } => Ok(local_addr),
            TunnelResponse::Error { message } => anyhow::bail!(message),
            other => anyhow::bail!("unexpected response: {:?}", other),
        }
    }

    pub(crate) async fn release_forward(&self, forward: ForwardSpec) -> anyhow::Result<bool> {
        let request = TunnelRequest::ReleaseForward {
            client_id: self.client_id.clone(),
            forward,
        };
        match self.send_request(request).await? {
            TunnelResponse::ReleaseForward { released } => Ok(released),
            TunnelResponse::Error { message } => anyhow::bail!(message),
            other => anyhow::bail!("unexpected response: {:?}", other),
        }
    }

    pub(crate) async fn heartbeat(&self) -> anyhow::Result<()> {
        let request = TunnelRequest::Heartbeat {
            client_id: self.client_id.clone(),
        };
        match self.send_request(request).await? {
            TunnelResponse::Ok => Ok(()),
            TunnelResponse::Error { message } => anyhow::bail!(message),
            other => anyhow::bail!("unexpected response: {:?}", other),
        }
    }

    pub(crate) async fn list_forwards(&self) -> anyhow::Result<Vec<ForwardStatus>> {
        match self.send_request(TunnelRequest::ListForwards).await? {
            TunnelResponse::Forwards { items } => Ok(items),
            TunnelResponse::Error { message } => anyhow::bail!(message),
            other => anyhow::bail!("unexpected response: {:?}", other),
        }
    }

    async fn send_request(&self, request: TunnelRequest) -> anyhow::Result<TunnelResponse> {
        let stream = timeout(TUNNEL_DAEMON_TIMEOUT, TcpStream::connect(&self.addr))
            .await
            .context("tunnel-daemon connect timeout")?
            .with_context(|| format!("failed to connect {}", self.addr))?;
        let mut framed = Framed::new(stream, LinesCodec::new());
        let payload = serde_json::to_string(&request)?;
        timeout(TUNNEL_DAEMON_TIMEOUT, framed.send(payload))
            .await
            .context("tunnel-daemon send timeout")??;
        let response = timeout(TUNNEL_DAEMON_TIMEOUT, framed.next())
            .await
            .context("tunnel-daemon read timeout")?;
        let response = response
            .context("tunnel-daemon closed connection")?
            .context("tunnel-daemon read error")?;
        let response: TunnelResponse = serde_json::from_str(&response)?;
        Ok(response)
    }
}
