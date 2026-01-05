pub(crate) mod app;
pub(crate) mod terminal;
pub(crate) mod theme;
mod format;
mod input;
mod render;
mod text;

pub(crate) use app::AppState;
pub(crate) use input::handle_key_event;
pub(crate) use render::draw_ui;
pub(crate) use terminal::{restore_terminal, setup_terminal};
