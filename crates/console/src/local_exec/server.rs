use std::collections::HashMap;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::Context;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use protocol::{CommandRequest, CommandResponse};

use super::ai_review::{AiReadonlyDecision, AiReadonlyReviewer};
use super::audit::{
    spawn_write_ai_review_record, spawn_write_request_record, spawn_write_request_record_value,
    AiReviewRecord, RequestRecord,
};
use super::events::{PendingRequest, ServerEvent};
use super::output::spawn_write_result_record;
use super::policy::{deny_message, request_summary, Whitelist};
use super::service::TargetServiceHandle;

pub(super) async fn spawn_command_server(
    listen_addr: SocketAddr,
    services: HashMap<String, TargetServiceHandle>,
    whitelist: Arc<Whitelist>,
    auto_approve_allowed: bool,
    ai_readonly_reviewer: Arc<Option<AiReadonlyReviewer>>,
    aggressive_mode: Arc<AtomicBool>,
    aggressive_clients: Arc<RwLock<HashSet<String>>>,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(listen_addr).await.map_err(|err| {
        anyhow::anyhow!("failed to bind command listener {}: {}", listen_addr, err)
    })?;
    let services = Arc::new(services);
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let services = Arc::clone(&services);
                    let whitelist = Arc::clone(&whitelist);
                    let ai_readonly_reviewer = Arc::clone(&ai_readonly_reviewer);
                    let aggressive_mode = Arc::clone(&aggressive_mode);
                    let aggressive_clients = Arc::clone(&aggressive_clients);
                    tokio::spawn(async move {
                        if let Err(err) = handle_connection(
                            stream,
                            addr,
                            services,
                            whitelist,
                            auto_approve_allowed,
                            ai_readonly_reviewer,
                            aggressive_mode,
                            aggressive_clients,
                        )
                        .await
                        {
                            tracing::error!(
                                event = "command.conn.error",
                                peer = %addr,
                                error = %err,
                                "command connection failed"
                            );
                        }
                    });
                }
                Err(err) => {
                    tracing::error!(
                        event = "command.listener.accept_failed",
                        error = %err,
                        "command listener accept failed"
                    );
                }
            }
        }
    });
    Ok(())
}

async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    services: Arc<HashMap<String, TargetServiceHandle>>,
    whitelist: Arc<Whitelist>,
    auto_approve_allowed: bool,
    ai_readonly_reviewer: Arc<Option<AiReadonlyReviewer>>,
    aggressive_mode: Arc<AtomicBool>,
    aggressive_clients: Arc<RwLock<HashSet<String>>>,
) -> anyhow::Result<()> {
    tracing::info!(event = "command.conn.open", peer = %addr);
    let codec = LengthDelimitedCodec::builder()
        .max_frame_length(protocol::framing::MAX_FRAME_LENGTH)
        .new_codec();
    let mut framed = Framed::new(stream, codec);
    while let Some(frame) = framed.next().await {
        let bytes = frame.context("frame read")?;
        let request: CommandRequest = match serde_json::from_slice(&bytes) {
            Ok(request) => request,
            Err(err) => {
                tracing::warn!(
                    event = "command.request.invalid",
                    peer = %addr,
                    error = %err,
                    "invalid request payload"
                );
                let response = CommandResponse::error("invalid", "invalid request");
                let payload = serde_json::to_vec(&response)?;
                let _ = framed.send(Bytes::from(payload)).await;
                continue;
            }
        };

        let handle = match services.get(&request.target) {
            Some(handle) => handle.clone(),
            None => {
                let response = CommandResponse::error(
                    request.id.clone(),
                    format!("unknown target {}", request.target),
                );
                let payload = serde_json::to_vec(&response)?;
                let _ = framed.send(Bytes::from(payload)).await;
                continue;
            }
        };

        tracing::info!(
            event = "command.request_received",
            id = %request.id,
            client = %request.client,
            target = %request.target,
            peer = %addr,
            command = %request_summary(&request),
        );

        let is_aggressive_mode = if aggressive_mode.load(Ordering::Relaxed) {
            true
        } else {
            let guard = aggressive_clients.read().await;
            guard.contains(&request.client)
        };
        if !is_aggressive_mode {
            if let Some(message) = deny_message(&whitelist, &request) {
                tracing::info!(
                    event = "command.request_denied_policy",
                    id = %request.id,
                    client = %request.client,
                    peer = %addr,
                    reason = %message,
                );
                let output_dir = Arc::clone(&handle.output_dir);
                let received_at = SystemTime::now();
                let record = RequestRecord::from_request(&request, &addr.to_string(), received_at);
                spawn_write_request_record_value(Arc::clone(&output_dir), record);
                let response = CommandResponse::denied(
                    request.id.clone(),
                    format!("denied by policy: {message}"),
                );
                spawn_write_result_record(
                    Arc::clone(&output_dir),
                    response.clone(),
                    Duration::from_secs(0),
                );
                let payload = serde_json::to_vec(&response)?;
                let _ = framed.send(Bytes::from(payload)).await;
                continue;
            }
        } else {
            tracing::info!(
                event = "command.request_aggressive_mode_override",
                id = %request.id,
                client = %request.client,
                peer = %addr,
                "policy checks bypassed by aggressive mode"
            );
        }

        let (respond_to, response_rx) = tokio::sync::oneshot::channel();
        let pending = PendingRequest {
            request,
            target_host: handle.target_host.clone(),
            target_desc: handle.target_desc.clone(),
            peer: addr.to_string(),
            received_at: SystemTime::now(),
            queued_at: Instant::now(),
            aggressive_mode: is_aggressive_mode,
            respond_to,
        };
        spawn_write_request_record(Arc::clone(&handle.output_dir), &pending);
        let should_auto_approve = if is_aggressive_mode {
            true
        } else {
            auto_approve_allowed
                && evaluate_auto_approve(
                    &pending.request,
                    &whitelist,
                    ai_readonly_reviewer.as_ref(),
                    Arc::clone(&handle.output_dir),
                )
                .await
        };
        let server_event = if should_auto_approve {
            tracing::info!(
                event = "command.request_auto_approved",
                id = %pending.request.id,
                target = %pending.request.target,
                client = %pending.request.client,
                peer = %addr,
                command = %request_summary(&pending.request),
            );
            ServerEvent::AutoApproveRequest(pending)
        } else {
            ServerEvent::Request(pending)
        };
        if handle.server_tx.send(server_event).await.is_err() {
            break;
        }

        match response_rx.await {
            Ok(response) => {
                let payload = serde_json::to_vec(&response)?;
                framed.send(Bytes::from(payload)).await?;
            }
            Err(_) => break,
        }
    }
    tracing::info!(event = "command.conn.closed", peer = %addr);
    Ok(())
}

async fn evaluate_auto_approve(
    request: &CommandRequest,
    whitelist: &Whitelist,
    ai_readonly_reviewer: &Option<AiReadonlyReviewer>,
    output_dir: Arc<std::path::PathBuf>,
) -> bool {
    if !whitelist.allows_request(request) {
        return false;
    }

    let Some(reviewer) = ai_readonly_reviewer else {
        return true;
    };

    match reviewer.review(request).await {
        Ok(decision) => {
            let approved = decision.read_only && decision.confidence >= reviewer.min_confidence();
            write_ai_review_record(
                output_dir,
                request.id.clone(),
                reviewer.endpoint().to_string(),
                reviewer.model().to_string(),
                reviewer.min_confidence(),
                decision.clone(),
                approved,
            );
            if approved {
                tracing::info!(
                    event = "command.request_ai_readonly_approved",
                    id = %request.id,
                    confidence = decision.confidence,
                    min_confidence = reviewer.min_confidence(),
                );
            } else {
                tracing::info!(
                    event = "command.request_ai_readonly_manual_review",
                    id = %request.id,
                    confidence = decision.confidence,
                    min_confidence = reviewer.min_confidence(),
                    "ai review did not reach approval threshold"
                );
            }
            approved
        }
        Err(err) => {
            tracing::warn!(
                event = "command.request_ai_readonly_failed",
                id = %request.id,
                error = %err,
                "ai readonly review failed, falling back to manual review"
            );
            false
        }
    }
}

fn write_ai_review_record(
    output_dir: Arc<std::path::PathBuf>,
    request_id: String,
    endpoint: String,
    model: String,
    min_confidence: f64,
    decision: AiReadonlyDecision,
    auto_approved: bool,
) {
    let record = AiReviewRecord {
        id: request_id,
        endpoint,
        model,
        min_confidence,
        read_only: decision.read_only,
        confidence: decision.confidence,
        reason: decision.reason,
        risk_flags: decision.risk_flags,
        auto_approved,
        reviewed_at_ms: now_ms(),
    };
    spawn_write_ai_review_record(output_dir, record);
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

impl Clone for TargetServiceHandle {
    fn clone(&self) -> Self {
        Self {
            server_tx: self.server_tx.clone(),
            command_tx: self.command_tx.clone(),
            snapshot: self.snapshot.clone(),
            output_dir: self.output_dir.clone(),
            target_host: self.target_host.clone(),
            target_desc: self.target_desc.clone(),
        }
    }
}
