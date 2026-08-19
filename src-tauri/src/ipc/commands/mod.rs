pub mod ai_cmd;
pub mod autostart_cmd;
pub mod cleanup_cmd;
pub mod config_cmd;
pub mod history_cmd;
pub mod hw_cmd;
pub mod stats_cmd;
pub mod system_cmd;
pub mod window_cmd;

#[cfg(not(test))]
pub fn export_bindings() {
    let bindings_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/bindings.ts");
    let typescript = specta_typescript::Typescript::default()
        .header("// @ts-nocheck")
        .bigint(specta_typescript::BigIntExportBehavior::Number);
    builder()
        .export(typescript, &bindings_path)
        .expect("failed to export TypeScript IPC bindings");
}

#[cfg(not(test))]
pub fn builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new()
        .error_handling(tauri_specta::ErrorHandlingMode::Throw)
        .typ::<crate::hw::client::HelperStatusEvent>()
        .commands(tauri_specta::collect_commands![
            ai_cmd::ai_get_mcp_overview,
            ai_cmd::ai_get_mcp_servers,
            ai_cmd::ai_discover_mcp_servers,
            ai_cmd::ai_toggle_mcp_server,
            ai_cmd::ai_delete_mcp_server,
            ai_cmd::ai_copy_mcp_server,
            ai_cmd::ai_mcp_oauth_status,
            ai_cmd::ai_mcp_oauth_authorize,
            ai_cmd::ai_mcp_oauth_cancel,
            ai_cmd::ai_mcp_oauth_refresh,
            ai_cmd::ai_mcp_oauth_revoke,
            ai_cmd::ai_list_session_tree,
            ai_cmd::ai_load_session,
            ai_cmd::ai_delete_session,
            ai_cmd::ai_delete_workspace,
            ai_cmd::ai_export_session,
            ai_cmd::ai_reveal_session_file,
            ai_cmd::ai_refresh_session_cache,
            config_cmd::get_config,
            config_cmd::set_config,
            config_cmd::reset_config,
            config_cmd::get_overlay_config,
            stats_cmd::get_realtime_stats,
            history_cmd::query_traffic_history,
            history_cmd::export_traffic_csv,
            system_cmd::get_app_info,
            system_cmd::open_path,
            system_cmd::quit_app,
            window_cmd::show_config_window,
            window_cmd::hide_config_window,
            window_cmd::resize_overlay,
            window_cmd::dock_overlay_to_taskbar,
            window_cmd::register_overlay_tooltip_regions,
            hw_cmd::get_hw_snapshot,
            hw_cmd::get_helper_status,
            hw_cmd::get_fan_control_state,
            hw_cmd::set_fan_manual,
            hw_cmd::set_fan_curve,
            hw_cmd::reset_fan_control,
            hw_cmd::reset_all_fan_controls,
            hw_cmd::is_admin,
            autostart_cmd::autostart_is_enabled,
            autostart_cmd::autostart_enable,
            autostart_cmd::autostart_disable,
            cleanup_cmd::scan_cleanup,
            cleanup_cmd::cancel_cleanup_scan,
            cleanup_cmd::export_cleanup_scan,
            cleanup_cmd::clean_categories,
            cleanup_cmd::scan_large_files,
        ])
}

#[cfg(test)]
pub fn handler() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        ai_cmd::ai_get_mcp_overview,
        ai_cmd::ai_get_mcp_servers,
        ai_cmd::ai_discover_mcp_servers,
        ai_cmd::ai_toggle_mcp_server,
        ai_cmd::ai_delete_mcp_server,
        ai_cmd::ai_copy_mcp_server,
        ai_cmd::ai_mcp_oauth_status,
        ai_cmd::ai_mcp_oauth_authorize,
        ai_cmd::ai_mcp_oauth_cancel,
        ai_cmd::ai_mcp_oauth_refresh,
        ai_cmd::ai_mcp_oauth_revoke,
        ai_cmd::ai_list_session_tree,
        ai_cmd::ai_load_session,
        ai_cmd::ai_delete_session,
        ai_cmd::ai_delete_workspace,
        ai_cmd::ai_export_session,
        ai_cmd::ai_reveal_session_file,
        ai_cmd::ai_refresh_session_cache,
        config_cmd::get_config,
        config_cmd::set_config,
        config_cmd::reset_config,
        config_cmd::get_overlay_config,
        stats_cmd::get_realtime_stats,
        history_cmd::query_traffic_history,
        history_cmd::export_traffic_csv,
        system_cmd::get_app_info,
        system_cmd::open_path,
        system_cmd::quit_app,
        window_cmd::show_config_window,
        window_cmd::hide_config_window,
        window_cmd::resize_overlay,
        window_cmd::dock_overlay_to_taskbar,
        window_cmd::register_overlay_tooltip_regions,
        hw_cmd::get_hw_snapshot,
        hw_cmd::get_helper_status,
        hw_cmd::get_fan_control_state,
        hw_cmd::set_fan_manual,
        hw_cmd::set_fan_curve,
        hw_cmd::reset_fan_control,
        hw_cmd::reset_all_fan_controls,
        hw_cmd::is_admin,
        autostart_cmd::autostart_is_enabled,
        autostart_cmd::autostart_enable,
        autostart_cmd::autostart_disable,
        cleanup_cmd::scan_cleanup,
        cleanup_cmd::cancel_cleanup_scan,
        cleanup_cmd::export_cleanup_scan,
        cleanup_cmd::clean_categories,
        cleanup_cmd::scan_large_files,
    ]
}
