use super::{ExecutionOutcome, ExecutionResult};

pub(super) fn build_execution_outcome(
    exit_code: Option<i32>,
    stdout_bytes: Vec<u8>,
    stdout_truncated: bool,
    stderr_chunks: Vec<(Vec<u8>, bool)>,
    cancelled: bool,
) -> ExecutionOutcome {
    let stdout = format_output(&stdout_bytes, stdout_truncated);
    let mut stderr = String::new();
    for (bytes, truncated) in stderr_chunks {
        append_output(&mut stderr, &bytes, truncated);
    }
    let stderr = if stderr.is_empty() {
        None
    } else {
        Some(stderr)
    };
    let result = ExecutionResult {
        exit_code,
        stdout,
        stderr,
    };
    if cancelled {
        ExecutionOutcome::Cancelled(result)
    } else {
        ExecutionOutcome::Completed(result)
    }
}

fn format_output(bytes: &[u8], truncated: bool) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let mut out = String::from_utf8_lossy(bytes).to_string();
    if truncated {
        out.push_str("\n[output truncated]");
    }
    Some(out)
}

fn append_output(target: &mut String, bytes: &[u8], truncated: bool) {
    if bytes.is_empty() {
        return;
    }
    if !target.is_empty() {
        target.push('\n');
    }
    target.push_str(&String::from_utf8_lossy(bytes));
    if truncated {
        target.push_str("\n[output truncated]");
    }
}
