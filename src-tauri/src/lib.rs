pub mod app;
pub mod config;
pub mod error;
pub mod hw;
pub mod ipc;
pub mod logging;
pub mod monitor;
pub mod paths;
pub mod sampler;
pub mod storage;
pub mod tray;

#[cfg(windows)]
pub mod windows_api;

pub use error::{AppError, IpcError, Result};

/// Application entry point invoked by the bin shim in `main.rs`.
pub fn run() {
    logging::init();
    tracing::info!("SysPulse starting up");

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
        .run(tauri::generate_context!());

    if let Err(e) = result {
        tracing::error!(error = %e, "tauri runtime exited with error");
        std::process::exit(1);
    }
}
