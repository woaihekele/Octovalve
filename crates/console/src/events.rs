use crate::state::TargetInfo;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ConsoleEvent {
    TargetsSnapshot { targets: Vec<TargetInfo> },
    TargetUpdated { target: TargetInfo },
}
