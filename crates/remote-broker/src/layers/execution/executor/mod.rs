mod pipeline;
mod output;
mod process;
mod shell;
mod stream;

use crate::layers::execution::output::write_result_record;
use crate::layers::policy::config::LimitsConfig;
use crate::layers::policy::whitelist::Whitelist;
use protocol::{CommandMode, CommandRequest, CommandResponse};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use self::pipeline::execute_pipeline;
use self::shell::execute_shell;

pub async fn execute_request(
    request: &CommandRequest,
    whitelist: &Whitelist,
    limits: &LimitsConfig,
    output_dir: &Path,
    cancel: CancellationToken,
) -> CommandResponse {
    let started_at = Instant::now();

    if cancel.is_cancelled() {
        let response = CommandResponse::cancelled(request.id.clone(), None, None, None);
        write_result_record(output_dir, &response, started_at.elapsed()).await;
        return response;
    }

    if matches!(&request.mode, CommandMode::Shell) && request.raw_command.trim().is_empty() {
        let response = CommandResponse::error(request.id.clone(), "raw_command is empty");
        write_result_record(output_dir, &response, started_at.elapsed()).await;
        return response;
    }

    if matches!(&request.mode, CommandMode::Argv) && request.pipeline.is_empty() {
        let response = CommandResponse::error(request.id.clone(), "pipeline is empty");
        write_result_record(output_dir, &response, started_at.elapsed()).await;
        return response;
    }

    if request.pipeline.is_empty() {
        let mode = &request.mode;
        tracing::warn!(
            id = %request.id,
            mode = ?mode,
            "empty pipeline, skipping whitelist validation"
        );
    } else {
        for stage in &request.pipeline {
            if let Err(message) = whitelist.validate_deny(stage) {
                let response = CommandResponse::denied(request.id.clone(), message);
                write_result_record(output_dir, &response, started_at.elapsed()).await;
                return response;
            }
        }
    }

    let timeout = Duration::from_secs(limits.timeout_secs);
    let max_bytes = usize::try_from(limits.max_output_bytes).unwrap_or(usize::MAX);
    let stdout_path = output_dir.join(format!("{}.stdout", request.id));
    let stderr_path = output_dir.join(format!("{}.stderr", request.id));

    let mut timed_out = false;
    let mut exec_fut = Box::pin(execute_command(
        request,
        max_bytes,
        &stdout_path,
        &stderr_path,
        cancel.clone(),
    ));
    let outcome = tokio::select! {
        result = &mut exec_fut => result,
        _ = tokio::time::sleep(timeout) => {
            timed_out = true;
            cancel.cancel();
            exec_fut.await
        }
    };

    let response = if timed_out {
        CommandResponse::error(request.id.clone(), "command timed out")
    } else {
        match outcome {
            Ok(ExecutionOutcome::Completed(result)) => CommandResponse::completed(
                request.id.clone(),
                result.exit_code.unwrap_or(1),
                result.stdout,
                result.stderr,
            ),
            Ok(ExecutionOutcome::Cancelled(result)) => CommandResponse::cancelled(
                request.id.clone(),
                result.exit_code,
                result.stdout,
                result.stderr,
            ),
            Err(err) => CommandResponse::error(request.id.clone(), err.to_string()),
        }
    };

    write_result_record(output_dir, &response, started_at.elapsed()).await;
    response
}

struct ExecutionResult {
    exit_code: Option<i32>,
    stdout: Option<String>,
    stderr: Option<String>,
}

enum ExecutionOutcome {
    Completed(ExecutionResult),
    Cancelled(ExecutionResult),
}

async fn execute_command(
    request: &CommandRequest,
    max_bytes: usize,
    stdout_path: &Path,
    stderr_path: &Path,
    cancel: CancellationToken,
) -> anyhow::Result<ExecutionOutcome> {
    match request.mode {
        CommandMode::Shell => {
            execute_shell(
                &request.raw_command,
                request.cwd.as_deref(),
                request.env.as_ref(),
                max_bytes,
                stdout_path,
                stderr_path,
                cancel,
            )
            .await
        }
        CommandMode::Argv => {
            execute_pipeline(
                &request.pipeline,
                request.cwd.as_deref(),
                request.env.as_ref(),
                max_bytes,
                stdout_path,
                stderr_path,
                cancel,
            )
            .await
        }
    }
}
