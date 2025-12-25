use crate::layers::service::events::ServiceEvent;
use crate::shared::dto::{RequestView, ResultView};
use ratatui::widgets::ListState;

pub(crate) const HISTORY_LIMIT: usize = 50;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ViewMode {
    #[default]
    Normal,
    ResultFullscreen,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ListView {
    #[default]
    Pending,
    History,
}

#[derive(Default)]
pub(crate) struct AppState {
    pub(crate) hostname: String,
    pub(crate) host_ip: String,
    pub(crate) queue: Vec<RequestView>,
    pub(crate) pending_selected: usize,
    pub(crate) pending_list_state: ListState,
    pub(crate) history: Vec<ResultView>,
    pub(crate) history_selected: usize,
    pub(crate) history_list_state: ListState,
    pub(crate) list_view: ListView,
    pub(crate) last_result: Option<ResultView>,
    pub(crate) view_mode: ViewMode,
    pub(crate) result_scroll: usize,
    pub(crate) result_max_scroll: usize,
    pub(crate) result_total_lines: usize,
    pub(crate) result_view_height: u16,
    pub(crate) pending_g: bool,
    pub(crate) confirm_quit: bool,
}

impl AppState {
    pub(crate) fn set_host_info(&mut self, hostname: String, host_ip: String) {
        self.hostname = hostname;
        self.host_ip = host_ip;
    }

    pub(crate) fn load_history(&mut self, mut history: Vec<ResultView>) {
        if history.len() > HISTORY_LIMIT {
            history.truncate(HISTORY_LIMIT);
        }
        self.history = history;
        self.last_result = self.history.first().cloned();
        self.history_selected = if self.history.is_empty() { 0 } else { 0 };
        self.sync_history_selection();
    }

    pub(crate) fn handle_event(&mut self, event: ServiceEvent) {
        match event {
            ServiceEvent::ConnectionsChanged => {}
            ServiceEvent::QueueUpdated(queue) => {
                let selected_id = self
                    .queue
                    .get(self.pending_selected)
                    .map(|item| item.id.clone());
                self.queue = queue;
                if let Some(id) = selected_id {
                    if let Some(pos) = self.queue.iter().position(|item| item.id == id) {
                        self.pending_selected = pos;
                    } else if !self.queue.is_empty() {
                        self.pending_selected = self.pending_selected.min(self.queue.len() - 1);
                    } else {
                        self.pending_selected = 0;
                    }
                } else if !self.queue.is_empty() {
                    self.pending_selected = self.pending_selected.min(self.queue.len() - 1);
                } else {
                    self.pending_selected = 0;
                }
                self.sync_pending_selection();
            }
            ServiceEvent::ResultUpdated(result) => {
                self.last_result = Some(result.clone());
                self.history.insert(0, result);
                if self.history.len() > HISTORY_LIMIT {
                    self.history.truncate(HISTORY_LIMIT);
                }
                if self.list_view == ListView::History {
                    self.history_selected = 0;
                } else if !self.history.is_empty() {
                    self.history_selected = self.history_selected.min(self.history.len() - 1);
                } else {
                    self.history_selected = 0;
                }
                self.sync_history_selection();
                self.result_scroll = 0;
                self.pending_g = false;
            }
        }
    }

    pub(crate) fn set_list_view(&mut self, view: ListView) {
        self.list_view = view;
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
        match self.list_view {
            ListView::Pending => {
                if self.queue.is_empty() {
                    return;
                }
                self.pending_selected = (self.pending_selected + 1) % self.queue.len();
                self.sync_pending_selection();
            }
            ListView::History => {
                if self.history.is_empty() {
                    return;
                }
                self.history_selected = (self.history_selected + 1) % self.history.len();
                self.sync_history_selection();
            }
        }
    }

    pub(crate) fn select_prev(&mut self) {
        match self.list_view {
            ListView::Pending => {
                if self.queue.is_empty() {
                    return;
                }
                if self.pending_selected == 0 {
                    self.pending_selected = self.queue.len() - 1;
                } else {
                    self.pending_selected -= 1;
                }
                self.sync_pending_selection();
            }
            ListView::History => {
                if self.history.is_empty() {
                    return;
                }
                if self.history_selected == 0 {
                    self.history_selected = self.history.len() - 1;
                } else {
                    self.history_selected -= 1;
                }
                self.sync_history_selection();
            }
        }
    }

    pub(crate) fn selected_request_id(&self) -> Option<String> {
        if self.list_view != ListView::Pending {
            return None;
        }
        self.queue
            .get(self.pending_selected)
            .map(|item| item.id.clone())
    }

    pub(crate) fn selected_history(&self) -> Option<&ResultView> {
        if self.list_view != ListView::History {
            return None;
        }
        self.history.get(self.history_selected)
    }

    fn sync_selection(&mut self) {
        self.sync_pending_selection();
        self.sync_history_selection();
    }

    fn sync_pending_selection(&mut self) {
        if self.queue.is_empty() {
            self.pending_list_state.select(None);
        } else {
            self.pending_list_state.select(Some(self.pending_selected));
        }
    }

    fn sync_history_selection(&mut self) {
        if self.history.is_empty() {
            self.history_list_state.select(None);
        } else {
            self.history_list_state.select(Some(self.history_selected));
        }
    }
}
