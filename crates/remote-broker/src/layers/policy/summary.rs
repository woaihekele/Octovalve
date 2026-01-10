use super::whitelist::Whitelist;
use protocol::{CommandMode, CommandRequest, CommandStage};

pub(crate) fn deny_message(whitelist: &Whitelist, request: &CommandRequest) -> Option<String> {
    for stage in &request.pipeline {
        if let Err(message) = whitelist.validate_deny(stage) {
            return Some(message);
        }
    }
    None
}

pub(crate) fn request_summary(request: &CommandRequest) -> String {
    let pipeline = format_pipeline(&request.pipeline);
    if pipeline.is_empty() {
        request.raw_command.clone()
    } else {
        pipeline
    }
}

pub(crate) fn format_mode(mode: &CommandMode) -> &'static str {
    match mode {
        CommandMode::Shell => "shell",
    }
}

pub(crate) fn format_pipeline(pipeline: &[CommandStage]) -> String {
    pipeline
        .iter()
        .map(|stage| stage.argv.join(" "))
        .collect::<Vec<_>>()
        .join(" | ")
}
