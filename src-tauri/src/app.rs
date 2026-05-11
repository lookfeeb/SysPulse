use crate::config::ConfigManager;
use crate::error::Result;
use crate::hw::{HwClient, HwSamplerHandle, HwSnapshot};
use crate::ipc::commands::window_cmd::{
    apply_overlay_config, dock_overlay_now, spawn_taskbar_overlay_z_order_watchdog,
};
use crate::ipc::events::{emit_config_changed, emit_overlay_config_changed, emit_stats};
use crate::monitor::Snapshot;
use crate::sampler::SamplerHandle;
use crate::storage::{spawn_writer, TrafficStore, WriterHandle};
use crate::{hw, logging, paths, sampler, storage, tray};
use parking_lot::RwLock;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{App, AppHandle, Emitter, Manager, WindowEvent};

/// State injected as Tauri managed state and consumed by IPC commands.
pub struct AppState {
    pub config: Arc<ConfigManager>,
    pub store: Arc<TrafficStore>,
    pub sampler: SamplerHandle,
    pub writer: WriterHandle,
    pub last_snapshot: Arc<RwLock<Option<Snapshot>>>,
    pub session: Arc<RwLock<SessionTraffic>>,
    pub hw_client: Arc<HwClient>,
    pub hw_sampler: HwSamplerHandle,
    pub last_hw_snapshot: Arc<RwLock<Option<HwSnapshot>>>,
    pub fan_control: hw::FanControlManager,
}

#[derive(Debug, Clone, Default)]
pub struct SessionTraffic {
    pub started_at: i64,
    pub bytes_recv: u64,
    pub bytes_sent: u64,
}

pub fn setup(app: &mut App) -> std::result::Result<(), Box<dyn std::error::Error>> {
    if let Err(e) = paths::ensure_dirs() {
        tracing::warn!(?e, "ensure_dirs failed");
    }

    // 1. Config
    let config = ConfigManager::load(paths::config_file())?;

    // 2. DB
    let store = TrafficStore::open(&paths::db_file())?;
    let _ = storage::queries::cleanup_old(store.pool(), config.snapshot().history.retain_days);
    spawn_retention_cleanup(store.pool().clone(), config.clone());

    // 3. DB writer task
    let writer = spawn_writer(Arc::new(store.pool().clone()));

    // 4. Sampler
    let sampler = sampler::spawn(config.clone(), writer.clone());

    let session = Arc::new(RwLock::new(SessionTraffic {
        started_at: chrono::Local::now().timestamp_millis(),
        ..Default::default()
    }));
    let last_snapshot = Arc::new(RwLock::new(None));

    // 5. Forward broadcast snapshots → webview events + tray + state cache
    {
        let app_handle = app.handle().clone();
        let mut rx = sampler.subscribe();
        let last_snapshot = last_snapshot.clone();
        let session = session.clone();
        tauri::async_runtime::spawn(async move {
            while let Ok(snap) = rx.recv().await {
                // Update session counters from per-second deltas (bytes)
                {
                    let mut s = session.write();
                    let interval_ms = snap.network.sample_interval_ms.max(1) as u64;
                    let secs = interval_ms as f64 / 1000.0;
                    let drecv = (snap.network.total.bytes_recv_per_sec as f64 * secs) as u64;
                    let dsent = (snap.network.total.bytes_sent_per_sec as f64 * secs) as u64;
                    s.bytes_recv = s.bytes_recv.saturating_add(drecv);
                    s.bytes_sent = s.bytes_sent.saturating_add(dsent);
                }
                *last_snapshot.write() = Some(snap.clone());
                emit_stats(&app_handle, &snap);
                tray::maybe_update_tray_stats(&app_handle, &snap);
            }
        });
    }

    // 6. Centralize config side effects: frontend notifications and overlay state.
    {
        let app_handle = app.handle().clone();
        let mut cfg_rx = config.subscribe();
        let mut prev_overlay = Some(config.snapshot().overlay);
        tauri::async_runtime::spawn(async move {
            while let Ok(cfg) = cfg_rx.recv().await {
                emit_config_changed(&app_handle, &cfg);
                if Some(&cfg.overlay) != prev_overlay.as_ref() {
                    if let Err(e) = apply_overlay_config(&app_handle, &cfg.overlay) {
                        tracing::warn!(?e, "apply overlay config failed");
                    }
                    emit_overlay_config_changed(&app_handle, &cfg);
                    // Re-dock after the webview applies new sizing.
                    let app_clone = app_handle.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                        if let Err(e) = dock_overlay_now(&app_clone) {
                            tracing::warn!(?e, "redock after config change failed");
                        }
                    });
                    prev_overlay = Some(cfg.overlay.clone());
                }
            }
        });
    }

    // 7. Tray
    if let Err(e) = tray::build(app.handle()) {
        tracing::warn!(?e, "tray build failed");
    }

    // 8. Hardware monitoring helper subprocess + sampler
    let last_hw_snapshot = Arc::new(RwLock::new(None));
    let helper_path = resolve_helper_path(app);
    let hw_client = Arc::new(HwClient::new(helper_path.clone()));
    hw_client.start();

    {
        let app_handle = app.handle().clone();
        let mut status_rx = hw_client.subscribe_status();
        tauri::async_runtime::spawn(async move {
            while let Ok(ev) = status_rx.recv().await {
                let _ = app_handle.emit("hw:helper-status", &ev);
            }
        });
    }

    let hw_sampler = hw::sampler::spawn(hw_client.clone());

    {
        let app_handle = app.handle().clone();
        let mut hw_rx = hw_sampler.subscribe();
        let last_hw = last_hw_snapshot.clone();
        tauri::async_runtime::spawn(async move {
            while let Ok(snap) = hw_rx.recv().await {
                *last_hw.write() = Some(snap.clone());
                let _ = app_handle.emit("hw:update", &snap);
            }
        });
    }

    if !helper_path.exists() {
        tracing::warn!(path = %helper_path.display(),
            "hw-helper.exe not found; run scripts/build-helper.ps1 first");
    }

    let fan_control = hw::FanControlManager::new();
    hw::fan_control::spawn_watchdog(
        app.handle().clone(),
        fan_control.clone(),
        hw_client.clone(),
        last_hw_snapshot.clone(),
    );

    // 9. Manage state for commands before showing the overlay, so its webview
    // can immediately call IPC during startup.
    app.manage(AppState {
        config,
        store,
        sampler,
        writer,
        last_snapshot,
        session,
        hw_client,
        hw_sampler,
        last_hw_snapshot,
        fan_control,
    });

    // 10. Apply initial overlay window state (visibility / position)
    if let Some(state) = app.try_state::<AppState>() {
        apply_initial_window_state(app, &state.config.snapshot());
    }
    spawn_taskbar_overlay_z_order_watchdog(app.handle().clone());

    #[cfg(windows)]
    {
        tauri::async_runtime::spawn(async {
            for delay_ms in [200_u64, 1200] {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                crate::windows_api::internal_windows::hide_auxiliary_windows();
            }
        });
    }

    tracing::info!("setup complete");
    Ok(())
}

fn spawn_retention_cleanup(
    pool: storage::store::DbPool,
    config: Arc<ConfigManager>,
) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(24 * 60 * 60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let retain_days = config.snapshot().history.retain_days;
            let pool = pool.clone();
            if let Err(e) = tokio::task::spawn_blocking(move || {
                storage::queries::cleanup_old(&pool, retain_days)
            })
            .await
            .unwrap_or_else(|e| Err(crate::error::AppError::Other(format!("join: {e}"))))
            {
                tracing::warn!(?e, "traffic history cleanup failed");
            }
            logging::cleanup_old_logs(logging::LOG_RETAIN_DAYS);
        }
    });
}

/// Find `hw-helper.exe` in (a) packaged Tauri resources or (b) the dev
/// `src-tauri/resources/hw-helper/` produced by `scripts/build-helper.ps1`.
fn resolve_helper_path(app: &App) -> PathBuf {
    if let Ok(resource_dir) = app.path().resource_dir() {
        let p = resource_dir.join("hw-helper").join("hw-helper.exe");
        if p.exists() {
            return p;
        }
    }
    // dev fallback: walk up from CARGO_MANIFEST_DIR
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let p = manifest_dir
        .join("resources")
        .join("hw-helper")
        .join("hw-helper.exe");
    if p.exists() {
        return p;
    }
    // Last resort: workspace root + resources/...
    let workspace = manifest_dir.parent().unwrap_or(&manifest_dir);
    workspace
        .join("src-tauri")
        .join("resources")
        .join("hw-helper")
        .join("hw-helper.exe")
}

fn apply_initial_window_state(app: &App, _cfg: &crate::config::AppConfig) {
    if let Some(w) = app.get_webview_window("overlay") {
        // Taskbar-docked mode: always_on_top is managed by dock logic
        let _ = w.set_always_on_top(false);

        let _ = w.show();
        let _ = w.set_ignore_cursor_events(false);

        // Defer until after the webview reports its settled size.
        let app_handle = app.handle().clone();
        tauri::async_runtime::spawn(async move {
            // Wait for HTML/CSS layout to finish so window outer_size is meaningful.
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            if let Err(e) = dock_overlay_now(&app_handle) {
                tracing::warn!(?e, "initial dock_overlay failed");
            }
            // On cold boot (auto-start) the webview may still be loading; retry
            // after a longer delay to fix the "squished" layout.
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            if let Err(e) = dock_overlay_now(&app_handle) {
                tracing::debug!(?e, "second dock_overlay attempt failed");
            }
        });
    }
}

pub fn on_second_instance(app: &AppHandle) -> Result<()> {
    if let Some(w) = app.get_webview_window("config") {
        let _ = w.show();
        let _ = w.set_focus();
    }
    Ok(())
}

pub fn request_quit(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        quit_gracefully(app).await;
    });
}

pub async fn quit_gracefully(app: AppHandle) {
    let cleanup = app.try_state::<AppState>().map(|state| {
        (
            state.fan_control.clone(),
            state.hw_client.clone(),
            state.writer.clone(),
            state.sampler.clone(),
            state.hw_sampler.clone(),
        )
    });

    if let Some((fan_control, hw_client, writer, sampler, hw_sampler)) = cleanup {
        let result = tokio::time::timeout(std::time::Duration::from_secs(4), async move {
            crate::hw::fan_control::reset_all_best_effort(&fan_control, &hw_client).await;
            writer.flush().await;
            sampler.shutdown().await;
            hw_sampler.shutdown().await;
            hw_client.shutdown().await;
        })
        .await;

        if result.is_err() {
            tracing::warn!("quit cleanup timed out; forcing app exit");
        }
    }

    app.exit(0);
}

pub fn on_window_event(window: &tauri::Window, event: &WindowEvent) {
    if window.label() != "config" {
        return;
    }
    if let WindowEvent::CloseRequested { api, .. } = event {
        // Hide instead of closing — keeps app running in tray.
        api.prevent_close();
        let _ = window.hide();
    }
}
