use crate::layers::service::events::ServiceEvent;
use crate::shared::dto::{RequestView, ResultView};
use ratatui::widgets::ListState;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ViewMode {
    #[default]
    Normal,
    ResultFullscreen,
}

#[derive(Default)]
pub(crate) struct AppState {
    pub(crate) queue: Vec<RequestView>,
    pub(crate) selected: usize,
    pub(crate) list_state: ListState,
    pub(crate) connections: usize,
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
    pub(crate) fn handle_event(&mut self, event: ServiceEvent) {
        match event {
            ServiceEvent::ConnectionsChanged(count) => {
                self.connections = count;
            }
            ServiceEvent::QueueUpdated(queue) => {
                let selected_id = self
                    .queue
                    .get(self.selected)
                    .map(|item| item.id.clone());
                self.queue = queue;
                if let Some(id) = selected_id {
                    if let Some(pos) = self.queue.iter().position(|item| item.id == id) {
                        self.selected = pos;
                    } else if !self.queue.is_empty() {
                        self.selected = self.selected.min(self.queue.len() - 1);
                    } else {
                        self.selected = 0;
                    }
                } else if !self.queue.is_empty() {
                    self.selected = self.selected.min(self.queue.len() - 1);
                } else {
                    self.selected = 0;
                }
            }
            ServiceEvent::ResultUpdated(result) => {
                self.last_result = Some(result);
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

    pub(crate) fn selected_request_id(&self) -> Option<String> {
        self.queue.get(self.selected).map(|item| item.id.clone())
    }

    fn sync_selection(&mut self) {
        if self.queue.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(self.selected));
        }
    }
}
