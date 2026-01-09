use std::sync::Arc;

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Mutex;

use crate::app_server::{AppServerClient, AppServerEvent};
use crate::cli::CliConfig;
use crate::handlers::{handle_acp_request, handle_app_server_stderr_line, handle_codex_event};
use crate::protocol::AcpMessage;
use crate::state::AcpState;
use crate::writer::AcpWriter;

pub async fn run_stdio() -> Result<()> {
    let config = CliConfig::parse()?;
    run_with_io(config, tokio::io::stdin(), tokio::io::stdout()).await
}

pub async fn run_with_io<R, W>(config: CliConfig, reader: R, writer: W) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    let writer = Arc::new(AcpWriter::new(Box::new(writer)));
    let state = Arc::new(Mutex::new(AcpState::default()));
    let (app_server, mut app_events) = AppServerClient::spawn(&config).await?;
    let app_server = Arc::new(app_server);

    let writer_clone = writer.clone();
    let state_clone = state.clone();
    tokio::spawn(async move {
        while let Some(event) = app_events.recv().await {
            match event {
                AppServerEvent::SessionConfigured { session_id } => {
                    let mut guard = state_clone.lock().await;
                    guard.session_id = Some(session_id.clone());
                    guard.saw_message_delta = false;
                    guard.saw_reasoning_delta = false;
                    for waiter in guard.session_id_waiters.drain(..) {
                        let _ = waiter.send(session_id.clone());
                    }
                }
                AppServerEvent::CodexEvent(msg) => {
                    if let Err(err) = handle_codex_event(msg, &writer_clone, &state_clone).await {
                        eprintln!("[acp-codex] 处理 codex 事件失败: {err}");
                    }
                }
                AppServerEvent::StderrLine(line) => {
                    if let Err(err) =
                        handle_app_server_stderr_line(line, &writer_clone, &state_clone).await
                    {
                        eprintln!("[acp-codex] 处理 app-server stderr 失败: {err}");
                    }
                }
            }
        }
    });

    let mut reader = BufReader::new(reader);
    let mut buffer = String::new();

    loop {
        buffer.clear();
        let bytes = reader.read_line(&mut buffer).await?;
        if bytes == 0 {
            break;
        }
        let line = buffer.trim_end_matches(['\n', '\r']);
        if line.is_empty() {
            continue;
        }

        let message: AcpMessage = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(err) => {
                eprintln!("[acp-codex] 无法解析 ACP 消息: {err}");
                continue;
            }
        };

        if let AcpMessage::Request(request) = message {
            if let Err(err) =
                handle_acp_request(request, &writer, &state, &app_server, &config).await
            {
                eprintln!("[acp-codex] 处理 ACP 请求失败: {err}");
            }
        }
    }

    Ok(())
}
