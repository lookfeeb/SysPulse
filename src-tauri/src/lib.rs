mod ai;
mod app;
mod config;
mod error;
mod hw;
mod ipc;
mod logging;
mod monitor;
mod paths;
mod sampler;
mod storage;
mod tray;

#[cfg(windows)]
mod windows_api;

#[cfg(not(test))]
pub use ipc::commands::export_bindings;

/// Application entry point invoked by the bin shim in `main.rs`.
pub fn run() {
    logging::init();
    tracing::info!(
        pid = std::process::id(),
        version = env!("CARGO_PKG_VERSION"),
        "SysPulse starting up"
    );
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(thread = ?std::thread::current().name(), location = ?info.location(),
            backtrace = %std::backtrace::Backtrace::force_capture(), "Rust panic");
        previous(info);
    }));

    let single_instance = tauri_plugin_single_instance::init(|app, _argv, _cwd| {
        if let Err(e) = app::on_second_instance(app) {
            tracing::warn!(?e, "on_second_instance handler failed");
        }
    });

    #[cfg(not(test))]
    let (ipc_builder, invoke_handler) = {
        let ipc_builder = ipc::commands::builder();
        let invoke_handler = ipc_builder.invoke_handler();
        (ipc_builder, invoke_handler)
    };

    let builder = tauri::Builder::default()
        .plugin(single_instance)
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build());

    #[cfg(not(test))]
    let builder = builder
        .setup(move |app| {
            ipc_builder.mount_events(app);
            app::setup(app)
        })
        .invoke_handler(invoke_handler);

    #[cfg(test)]
    let builder = builder
        .setup(app::setup)
        .invoke_handler(ipc::commands::handler());

    let result = builder
        .on_window_event(app::on_window_event)
        .build(tauri::generate_context!());

    match result {
        Ok(application) => application.run(|_, event| match event {
            tauri::RunEvent::ExitRequested { code, api, .. } => {
                let explicit = app::is_shutting_down();
                tracing::info!(?code, explicit, "runtime exit requested");
                // Losing native windows must not shut down a tray application.
                if code.is_none() && !explicit {
                    api.prevent_exit();
                }
                #[cfg(windows)]
                if let Some(code) = code.filter(|_| explicit) {
                    // Popup-menu shutdown can leave Tao waiting in GetMessageW
                    // after RequestExit with no visible windows. Wake its own
                    // message pump with WM_QUIT; LoopDestroyed and Tauri cleanup
                    // still run normally. Cleanup has already completed above.
                    unsafe {
                        windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(code);
                    }
                }
            }
            tauri::RunEvent::Exit => {
                #[cfg(windows)]
                windows_api::taskbar::stop_overlay_z_order_watch();
                tracing::info!("SysPulse runtime exited");
            }
            _ => {}
        }),
        Err(e) => {
            tracing::error!(error = %e, "tauri runtime failed to start");
            std::process::exit(1);
        }
    }
}
