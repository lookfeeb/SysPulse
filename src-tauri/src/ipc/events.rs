use crate::config::AppConfig;
use crate::monitor::Snapshot;
use tauri::{AppHandle, Emitter};

pub fn emit_stats(app: &AppHandle, snap: &Snapshot) {
    let _ = app.emit("stats:update", snap);
}

pub fn emit_config_changed(app: &AppHandle, cfg: &AppConfig) {
    let _ = app.emit("config:changed", cfg);
}

pub fn emit_overlay_config_changed(app: &AppHandle, cfg: &AppConfig) {
    let _ = app.emit_to("overlay", "overlay:config-changed", &cfg.overlay);
}
