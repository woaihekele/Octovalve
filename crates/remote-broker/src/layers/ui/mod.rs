pub(crate) mod app;
mod format;
mod input;
mod render;
pub(crate) mod terminal;
mod text;
pub(crate) mod theme;

pub(crate) use app::AppState;
pub(crate) use input::handle_key_event;
pub(crate) use render::draw_ui;
pub(crate) use terminal::{restore_terminal, setup_terminal};
