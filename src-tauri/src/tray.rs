use crate::app::AppState;
use crate::error::Result;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

const TRAY_ID: &str = "main";
const MENU_ID_OPEN_WINDOW: &str = "open_window";
const MENU_ID_QUIT: &str = "quit";

pub struct TrayState {
    pub last_tooltip_update_ms: AtomicI64,
}

impl TrayState {
    fn new() -> Self {
        Self {
            last_tooltip_update_ms: AtomicI64::new(0),
        }
    }
}

pub fn build(app: &AppHandle) -> Result<()> {
    let state = Arc::new(TrayState::new());
    app.manage(state.clone());

    let menu = build_menu(app)?;

    let app_for_tray_events = app.clone();
    let _tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(app.default_window_icon().cloned().unwrap())
        .tooltip("SysPulse")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(move |_tray, event| {
            handle_tray_icon_event(&app_for_tray_events, event);
        })
        .on_menu_event(handle_menu_event)
        .build(app)?;

    Ok(())
}

fn build_menu(app: &AppHandle) -> Result<Menu<tauri::Wry>> {
    let open = MenuItemBuilder::with_id(MENU_ID_OPEN_WINDOW, "打开窗口").build(app)?;
    let quit = MenuItemBuilder::with_id(MENU_ID_QUIT, "退出程序").build(app)?;

    let menu = MenuBuilder::new(app).item(&open).item(&quit).build()?;

    Ok(menu)
}

fn handle_tray_icon_event(app: &AppHandle, event: TrayIconEvent) {
    if let TrayIconEvent::DoubleClick {
        button: MouseButton::Left,
        ..
    } = event
    {
        show_config_window(app);
    }
}

fn handle_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    match event.id().as_ref() {
        MENU_ID_OPEN_WINDOW => {
            show_config_window(app);
        }
        MENU_ID_QUIT => {
            crate::app::request_quit(app.clone());
        }
        _ => {}
    }
}

fn show_config_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("config") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub fn maybe_update_tray_stats(app: &AppHandle, snap: &crate::monitor::Snapshot) {
    let now = chrono::Local::now().timestamp_millis();
    let Some(mgr) = app.try_state::<Arc<TrayState>>() else {
        return;
    };
    let prev = mgr.last_tooltip_update_ms.load(Ordering::Relaxed);
    if now - prev < 1000 {
        return;
    }
    mgr.last_tooltip_update_ms.store(now, Ordering::Relaxed);

    let hw = app
        .try_state::<AppState>()
        .and_then(|s| s.last_hw_snapshot.read().clone());

    let tooltip = build_tooltip(snap, hw.as_ref());

    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(tooltip));
    }
}

/// Build a compact tooltip that fits within the Windows 128-char limit.
///
/// Layout:
/// ```text
/// SysPulse
/// CPU 46°C 27% | 内存 14.2/32.0GB
/// ↓ 1.2 MB/s ↑ 80 KB/s
/// ```
fn build_tooltip(snap: &crate::monitor::Snapshot, hw: Option<&crate::hw::HwSnapshot>) -> String {
    let mut lines: Vec<String> = Vec::with_capacity(3);
    lines.push("SysPulse".into());

    // Line 2: CPU + Memory
    if let Some(h) = hw {
        let mut hw_parts: Vec<String> = Vec::new();

        if let Some(cpu) = &h.cpu {
            let temp = cpu
                .package_temp_c
                .map(|t| format!("{:.0}°C", t))
                .unwrap_or_default();
            hw_parts.push(format!("CPU {temp} {:.0}%", cpu.total_usage));
        }

        if let Some(mem) = &h.memory {
            if mem.total_bytes > 0 {
                let used_gb = mem.used_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                let total_gb = mem.total_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                hw_parts.push(format!("内存 {:.1}/{:.1}GB", used_gb, total_gb));
            }
        }

        if !hw_parts.is_empty() {
            lines.push(hw_parts.join(" | "));
        }
    }

    // Line 3: Network
    let down = format_speed(snap.network.total.bytes_recv_per_sec);
    let up = format_speed(snap.network.total.bytes_sent_per_sec);
    lines.push(format!("↓{} ↑{}", down, up));

    lines.join("\n")
}

fn format_speed(b_per_s: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;
    if b_per_s >= GB {
        format!("{:.1} GB/s", b_per_s as f64 / GB as f64)
    } else if b_per_s >= MB {
        format!("{:.1} MB/s", b_per_s as f64 / MB as f64)
    } else if b_per_s >= KB {
        format!("{:.1} KB/s", b_per_s as f64 / KB as f64)
    } else {
        format!("{} B/s", b_per_s)
    }
}
