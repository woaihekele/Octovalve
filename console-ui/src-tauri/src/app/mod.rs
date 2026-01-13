pub mod setup;
pub mod window_events;

pub fn run() {
    tauri::Builder::default()
        .manage(crate::state::ConsoleSidecarState(std::sync::Mutex::new(
            None,
        )))
        .manage(crate::state::ConsoleStreamState(std::sync::Mutex::new(
            false,
        )))
        .manage(crate::state::TerminalSessions(std::sync::Mutex::new(
            std::collections::HashMap::new(),
        )))
        .manage(crate::state::AppLanguageState(std::sync::Mutex::new(None)))
        .manage(crate::clients::AcpClientState::default())
        .manage(crate::clients::McpClientState::default())
        .manage(crate::clients::OpenAiClientState(tokio::sync::Mutex::new(
            None,
        )))
        .invoke_handler(tauri::generate_handler![
            crate::commands::profiles::list_profiles,
            crate::commands::profiles::create_profile,
            crate::commands::profiles::delete_profile,
            crate::commands::profiles::select_profile,
            crate::commands::profiles::read_profile_proxy_config,
            crate::commands::profiles::write_profile_proxy_config,
            crate::commands::profiles::read_profile_broker_config,
            crate::commands::profiles::write_profile_broker_config,
            crate::commands::profiles::get_proxy_config_status,
            crate::commands::config::read_proxy_config,
            crate::commands::config::write_proxy_config,
            crate::commands::config::parse_proxy_config_toml,
            crate::commands::config::parse_broker_config_toml,
            crate::commands::console::restart_console,
            crate::commands::console::validate_startup_config,
            crate::commands::console::log_ui_event,
            crate::commands::console::set_app_language,
            crate::commands::console::proxy_fetch_targets,
            crate::commands::console::proxy_fetch_snapshot,
            crate::commands::console::proxy_approve,
            crate::commands::console::proxy_deny,
            crate::commands::console::proxy_cancel,
            crate::commands::console::proxy_list_target_dirs,
            crate::commands::console::proxy_start_upload,
            crate::commands::console::proxy_upload_status,
            crate::commands::console::read_console_log,
            crate::commands::console::read_app_log,
            crate::commands::ai::ai_risk_assess,
            crate::commands::console::start_console_stream,
            crate::commands::terminal::terminal_open,
            crate::commands::terminal::terminal_input,
            crate::commands::terminal::terminal_resize,
            crate::commands::terminal::terminal_close,
            crate::commands::acp::acp_start,
            crate::commands::acp::acp_authenticate,
            crate::commands::acp::acp_new_session,
            crate::commands::acp::acp_load_session,
            crate::commands::acp::acp_list_sessions,
            crate::commands::acp::acp_delete_session,
            crate::commands::acp::acp_prompt,
            crate::commands::acp::acp_cancel,
            crate::commands::acp::acp_stop,
            crate::commands::openai::openai_init,
            crate::commands::openai::openai_add_message,
            crate::commands::openai::openai_set_tools,
            crate::commands::openai::openai_clear_messages,
            crate::commands::openai::openai_cancel,
            crate::commands::openai::openai_send,
            crate::commands::mcp::mcp_call_tool,
            crate::commands::opener::open_external
        ])
        .setup(|app| {
            crate::app::setup::init(app).map_err(|err| {
                Box::<dyn std::error::Error>::from(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    err,
                ))
            })
        })
        .plugin(tauri_plugin_shell::init())
        .on_window_event(crate::app::window_events::handle)
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(crate::app::window_events::handle_run);
}
