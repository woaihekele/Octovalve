use protocol::{CommandRequest, CommandResponse, CommandStatus};
use ratatui::widgets::ListState;
use std::time::{Instant, SystemTime};
use tokio::sync::oneshot;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ViewMode {
    #[default]
    Normal,
    ResultFullscreen,
}

#[derive(Default)]
pub(crate) struct AppState {
    pub(crate) queue: Vec<PendingRequest>,
    pub(crate) selected: usize,
    pub(crate) list_state: ListState,
    pub(crate) connections: usize,
    pub(crate) last_result: Option<ExecutionRecord>,
    pub(crate) view_mode: ViewMode,
    pub(crate) result_scroll: usize,
    pub(crate) result_max_scroll: usize,
    pub(crate) result_total_lines: usize,
    pub(crate) result_view_height: u16,
    pub(crate) pending_g: bool,
    pub(crate) confirm_quit: bool,
}

impl AppState {
    pub(crate) fn handle_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::ConnectionOpened => self.connections += 1,
            UiEvent::ConnectionClosed => {
                self.connections = self.connections.saturating_sub(1);
            }
            UiEvent::Request(pending) => {
                self.queue.push(pending);
                if self.queue.len() == 1 {
                    self.selected = 0;
                }
            }
            UiEvent::Execution(record) => {
                self.last_result = Some(record);
                self.result_scroll = 0;
                self.pending_g = false;
            }
        }
        self.sync_selection();
    }

    pub(crate) fn enter_result_fullscreen(&mut self) {
        self.view_mode = ViewMode::ResultFullscreen;
        self.result_scroll = 0;
        self.pending_g = false;
        self.confirm_quit = false;
    }

    pub(crate) fn exit_result_fullscreen(&mut self) {
        self.view_mode = ViewMode::Normal;
        self.pending_g = false;
    }

    pub(crate) fn set_result_metrics(&mut self, total_lines: usize, view_height: u16) {
        let total_lines = total_lines.max(1);
        self.result_total_lines = total_lines;
        self.result_view_height = view_height;
        self.result_max_scroll = total_lines.saturating_sub(view_height as usize);
        if self.result_scroll > self.result_max_scroll {
            self.result_scroll = self.result_max_scroll;
        }
    }

    pub(crate) fn scroll_down(&mut self, lines: usize) {
        self.result_scroll = (self.result_scroll + lines).min(self.result_max_scroll);
        self.pending_g = false;
    }

    pub(crate) fn scroll_up(&mut self, lines: usize) {
        self.result_scroll = self.result_scroll.saturating_sub(lines);
        self.pending_g = false;
    }

    pub(crate) fn scroll_to_top(&mut self) {
        self.result_scroll = 0;
        self.pending_g = false;
    }

    pub(crate) fn scroll_to_bottom(&mut self) {
        self.result_scroll = self.result_max_scroll;
        self.pending_g = false;
    }

    pub(crate) fn page_size(&self) -> usize {
        let height = self.result_view_height.max(1) as usize;
        height.saturating_sub(1).max(1)
    }

    pub(crate) fn half_page_size(&self) -> usize {
        let height = self.result_view_height.max(1) as usize;
        (height / 2).max(1)
    }

    pub(crate) fn select_next(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.queue.len();
        self.sync_selection();
    }

    pub(crate) fn select_prev(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        if self.selected == 0 {
            self.selected = self.queue.len() - 1;
        } else {
            self.selected -= 1;
        }
        self.sync_selection();
    }

    pub(crate) fn pop_selected(&mut self) -> Option<PendingRequest> {
        if self.queue.is_empty() {
            return None;
        }
        let index = self.selected.min(self.queue.len() - 1);
        let item = self.queue.remove(index);
        if self.selected >= self.queue.len() && !self.queue.is_empty() {
            self.selected = self.queue.len() - 1;
        }
        self.sync_selection();
        Some(item)
    }

    fn sync_selection(&mut self) {
        if self.queue.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(self.selected));
        }
    }
}

pub(crate) struct PendingRequest {
    pub(crate) request: CommandRequest,
    pub(crate) peer: String,
    pub(crate) received_at: SystemTime,
    pub(crate) queued_at: Instant,
    pub(crate) respond_to: oneshot::Sender<CommandResponse>,
}

pub(crate) struct ExecutionRecord {
    pub(crate) id: String,
    pub(crate) status: CommandStatus,
    pub(crate) exit_code: Option<i32>,
    pub(crate) summary: String,
    pub(crate) stdout: Option<String>,
    pub(crate) stderr: Option<String>,
}

impl ExecutionRecord {
    pub(crate) fn from_response(pending: &PendingRequest, response: &CommandResponse) -> Self {
        let summary = match response.status {
            CommandStatus::Completed => format!("completed (exit={:?})", response.exit_code),
            CommandStatus::Denied => "denied".to_string(),
            CommandStatus::Error => "error".to_string(),
            CommandStatus::Approved => "approved".to_string(),
        };
        Self {
            id: pending.request.id.clone(),
            status: response.status.clone(),
            exit_code: response.exit_code,
            summary,
            stdout: response.stdout.clone(),
            stderr: response.stderr.clone(),
        }
    }
}

pub(crate) enum UiEvent {
    ConnectionOpened,
    ConnectionClosed,
    Request(PendingRequest),
    Execution(ExecutionRecord),
}
