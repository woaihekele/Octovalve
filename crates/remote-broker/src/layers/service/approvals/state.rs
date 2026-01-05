use std::collections::HashMap;

use protocol::control::{ResultSnapshot, RunningSnapshot, ServiceSnapshot};
use tokio_util::sync::CancellationToken;

use crate::layers::service::events::PendingRequest;

use super::snapshots::build_queue_snapshots;

pub(crate) struct ServiceState {
    pub(crate) pending: Vec<PendingRequest>,
    pub(crate) running: Vec<RunningSnapshot>,
    running_tokens: HashMap<String, CancellationToken>,
    history: Vec<ResultSnapshot>,
    history_limit: usize,
}

impl ServiceState {
    pub(crate) fn new(history: Vec<ResultSnapshot>, history_limit: usize) -> Self {
        Self {
            pending: Vec::new(),
            running: Vec::new(),
            running_tokens: HashMap::new(),
            history,
            history_limit,
        }
    }

    pub(crate) fn start_running(&mut self, running: RunningSnapshot, token: CancellationToken) {
        self.running.retain(|item| item.id != running.id);
        self.running.insert(0, running.clone());
        self.running_tokens.insert(running.id, token);
    }

    pub(crate) fn finish_running(&mut self, id: &str) -> bool {
        let before = self.running.len();
        self.running.retain(|item| item.id != id);
        self.running_tokens.remove(id);
        before != self.running.len()
    }

    pub(crate) fn cancel_running(&mut self, id: &str) -> bool {
        if let Some(token) = self.running_tokens.get(id) {
            token.cancel();
            return true;
        }
        false
    }

    pub(crate) fn push_result(&mut self, result: ResultSnapshot) {
        self.history.insert(0, result);
        if self.history.len() > self.history_limit {
            self.history.truncate(self.history_limit);
        }
    }

    pub(crate) fn snapshot(&self) -> ServiceSnapshot {
        let queue = build_queue_snapshots(&self.pending);
        let last_result = self.history.first().cloned();
        ServiceSnapshot {
            queue,
            running: self.running.clone(),
            history: self.history.clone(),
            last_result,
        }
    }
}
