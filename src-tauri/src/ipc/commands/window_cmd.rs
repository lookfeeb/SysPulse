use crate::app::AppState;
use crate::config::OverlayConfig;
use crate::error::{AppError, IpcError};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, State};
#[cfg(windows)]
use windows::Win32::Foundation::RECT;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

const CONFIG_LABEL: &str = "config";
const OVERLAY_LABEL: &str = "overlay";

#[derive(Clone, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OverlayTooltipRegion {
    pub key: String,
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

#[derive(serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OverlayTooltipRegionsArgs {
    pub regions: Vec<OverlayTooltipRegion>,
}

#[cfg(windows)]
static OVERLAY_TOOLTIP_REGIONS: std::sync::OnceLock<std::sync::RwLock<Vec<OverlayTooltipRegion>>> =
    std::sync::OnceLock::new();

#[tauri::command]
#[specta::specta]
pub fn register_overlay_tooltip_regions(args: OverlayTooltipRegionsArgs) -> Result<(), IpcError> {
    #[cfg(windows)]
    {
        let regions = OVERLAY_TOOLTIP_REGIONS.get_or_init(Default::default);
        *regions.write().unwrap_or_else(|e| e.into_inner()) = args.regions;
    }
    #[cfg(not(windows))]
    let _ = args;
    Ok(())
}

pub fn spawn_overlay_tooltip_watchdog(app: AppHandle) {
    #[cfg(windows)]
    tauri::async_runtime::spawn(async move {
        use std::time::{Duration, Instant};
        use windows::Win32::Foundation::HWND;

        let mut interval = tokio::time::interval(Duration::from_millis(120));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut active_key: Option<String> = None;
        let mut last_refresh = Instant::now() - Duration::from_secs(2);

        loop {
            interval.tick().await;
            let hovered = overlay_tooltip_hit_test(&app);
            let next_key = hovered.as_ref().map(|hit| hit.key.as_str());
            let changed = next_key != active_key.as_deref();

            let Some(hit) = hovered else {
                if active_key.take().is_some() {
                    let _ = app.run_on_main_thread(crate::windows_api::native_tooltip::hide);
                }
                continue;
            };

            if !changed && last_refresh.elapsed() < Duration::from_secs(1) {
                continue;
            }
            let Some(state) = app.try_state::<AppState>() else {
                continue;
            };
            let config = state.config.snapshot();
            let snapshot = state.last_snapshot.read();
            let hw = state.last_hw_snapshot.read();
            let text = if hit.key == "__all__" {
                build_overlay_summary(&config.overlay.items, snapshot.as_ref(), hw.as_ref())
            } else {
                build_overlay_tooltip(&hit.key, snapshot.as_ref(), hw.as_ref())
            };
            let Some(text) = text else {
                continue;
            };

            let key = hit.key.clone();
            let owner_raw = hit.owner;
            let x = hit.x;
            let y = hit.y;
            let _ = app.run_on_main_thread(move || {
                let owner = HWND(owner_raw as *mut _);
                if let Err(error) = crate::windows_api::native_tooltip::show(owner, &text, x, y) {
                    tracing::warn!(?error, key = %key, "show native overlay tooltip failed");
                }
            });
            active_key = Some(hit.key);
            last_refresh = Instant::now();
        }
    });
    #[cfg(not(windows))]
    let _ = app;
}

#[cfg(windows)]
struct OverlayTooltipHit {
    key: String,
    owner: isize,
    x: i32,
    y: i32,
}

#[cfg(windows)]
fn overlay_tooltip_hit_test(app: &AppHandle) -> Option<OverlayTooltipHit> {
    use windows::Win32::Foundation::{HWND, POINT};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let window = app.get_webview_window(OVERLAY_LABEL)?;
    if !window.is_visible().ok()? {
        return None;
    }
    let hwnd = window.hwnd().ok()?;
    let owner = HWND(hwnd.0 as _);
    let mut window_rect = RECT::default();
    let mut cursor = POINT::default();
    unsafe {
        GetWindowRect(owner, &mut window_rect).ok()?;
        GetCursorPos(&mut cursor).ok()?;
    }
    if cursor.x < window_rect.left
        || cursor.x >= window_rect.right
        || cursor.y < window_rect.top
        || cursor.y >= window_rect.bottom
    {
        return None;
    }

    let scale = window.scale_factor().ok()?.max(0.1);
    let client_x = (cursor.x - window_rect.left) as f64 / scale;
    let client_y = (cursor.y - window_rect.top) as f64 / scale;
    let regions = OVERLAY_TOOLTIP_REGIONS.get_or_init(Default::default);
    let regions = regions.read().unwrap_or_else(|e| e.into_inner());
    let region = regions.iter().find(|region| {
        client_x >= region.left - 2.0
            && client_x <= region.right + 2.0
            && client_y >= region.top - 2.0
            && client_y <= region.bottom + 2.0
    })?;

    Some(OverlayTooltipHit {
        key: region.key.clone(),
        owner: owner.0 as isize,
        x: window_rect.left + (((region.left + region.right) / 2.0) * scale).round() as i32,
        y: window_rect.top + (region.top * scale).round() as i32,
    })
}

#[cfg(windows)]
fn build_overlay_summary(
    items: &[crate::config::OverlayItem],
    snapshot: Option<&crate::monitor::Snapshot>,
    hw: Option<&crate::hw::HwSnapshot>,
) -> Option<String> {
    use crate::config::OverlayItem;
    use std::collections::HashSet;

    let mut lines = Vec::new();
    let mut groups = HashSet::new();
    for item in items {
        let line = match item {
            OverlayItem::NetDown | OverlayItem::NetUp if groups.insert("network") => {
                let total = snapshot.map(|s| &s.network.total);
                Some(format!(
                    "网速：↓{}  ↑{}",
                    total
                        .map(|value| format_speed(value.bytes_recv_per_sec as f64))
                        .unwrap_or_else(|| "--".into()),
                    total
                        .map(|value| format_speed(value.bytes_sent_per_sec as f64))
                        .unwrap_or_else(|| "--".into())
                ))
            }
            OverlayItem::Cpu => Some(format!(
                "CPU 占用：{}",
                hw.and_then(|h| h.cpu.as_ref())
                    .map(|cpu| format_percent(cpu.total_usage))
                    .or_else(|| snapshot.map(|s| format_percent(s.cpu.usage_percent as f64)))
                    .unwrap_or_else(|| "--".into())
            )),
            OverlayItem::CpuTemp => Some(format!(
                "CPU 温度：{}",
                format_temp(
                    hw.and_then(|h| h.cpu.as_ref())
                        .and_then(|cpu| cpu.package_temp_c)
                )
            )),
            OverlayItem::CpuFreq => Some(format!(
                "CPU 频率：{}",
                format_frequency(
                    hw.and_then(|h| h.cpu.as_ref())
                        .and_then(|cpu| cpu.frequency_mhz)
                )
            )),
            OverlayItem::Mem => {
                let memory = hw.and_then(|h| h.memory.as_ref());
                let percent = memory
                    .map(|value| format_percent(value.used_percent))
                    .or_else(|| snapshot.map(|s| format_percent(s.memory.used_percent as f64)))
                    .unwrap_or_else(|| "--".into());
                Some(match memory {
                    Some(value) if value.total_bytes > 0 => format!(
                        "内存：{percent}（{} / {}）",
                        format_bytes(value.used_bytes as f64),
                        format_bytes(value.total_bytes as f64)
                    ),
                    _ => format!("内存：{percent}"),
                })
            }
            OverlayItem::Gpu | OverlayItem::GpuUsage => Some(format!(
                "GPU 占用：{}",
                format_percent_opt(max_value(
                    hw.into_iter()
                        .flat_map(|h| { h.gpus.iter().filter_map(|gpu| gpu.usage_percent) })
                ))
            )),
            OverlayItem::GpuTemp => Some(format!(
                "GPU 温度：{}",
                format_temp(max_value(
                    hw.into_iter()
                        .flat_map(|h| { h.gpus.iter().filter_map(|gpu| gpu.temp_c) })
                ))
            )),
            OverlayItem::DiskRead | OverlayItem::DiskWrite if groups.insert("disk-io") => {
                let disks = hw.map(|h| h.disks.as_slice()).unwrap_or_default();
                let read = sum_values(disks.iter().filter_map(|disk| disk.read_bytes_per_sec));
                let write = sum_values(disks.iter().filter_map(|disk| disk.write_bytes_per_sec));
                Some(format!(
                    "硬盘：↓{}  ↑{}",
                    format_speed_opt(read),
                    format_speed_opt(write)
                ))
            }
            OverlayItem::DiskTemp => Some(format!(
                "硬盘温度：{}",
                format_temp(max_value(
                    hw.into_iter()
                        .flat_map(|h| { h.disks.iter().filter_map(|disk| disk.temp_c) })
                ))
            )),
            OverlayItem::FanRpm => Some(format!(
                "风扇：{}",
                max_value(hw.into_iter().flat_map(|h| {
                    h.fans
                        .iter()
                        .filter_map(|fan| fan.rpm)
                        .filter(|rpm| *rpm > 0.0)
                }))
                .map(|rpm| format!("{rpm:.0} RPM"))
                .unwrap_or_else(|| "--".into())
            )),
            OverlayItem::MbTemp => Some(format!(
                "主板温度：{}",
                format_temp(max_value(hw.into_iter().flat_map(|h| {
                    h.motherboard
                        .iter()
                        .flat_map(|board| board.temperatures_c.iter().map(|temp| temp.value))
                })))
            )),
            _ => None,
        };
        if let Some(line) = line {
            lines.push(line);
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(format!("SysPulse\r\n{}", lines.join("\r\n")))
    }
}

#[cfg(windows)]
fn max_value(values: impl Iterator<Item = f64>) -> Option<f64> {
    values.filter(|value| value.is_finite()).reduce(f64::max)
}

#[cfg(windows)]
fn sum_values(values: impl Iterator<Item = f64>) -> Option<f64> {
    let values: Vec<f64> = values.filter(|value| value.is_finite()).collect();
    (!values.is_empty()).then(|| values.into_iter().sum())
}

#[cfg(windows)]
fn build_overlay_tooltip(
    key: &str,
    snapshot: Option<&crate::monitor::Snapshot>,
    hw: Option<&crate::hw::HwSnapshot>,
) -> Option<String> {
    let mut lines = Vec::new();
    let title = match key {
        "cpu" => {
            if let Some(cpu) = hw.and_then(|h| h.cpu.as_ref()) {
                lines.push(format!("当前占用：{}", format_percent(cpu.total_usage)));
                if !cpu.name.is_empty() {
                    lines.push(format!("型号：{}", cpu.name));
                }
                if let Some(power) = cpu.power_w {
                    lines.push(format!("功耗：{power:.1} W"));
                }
            } else if let Some(s) = snapshot {
                lines.push(format!(
                    "当前占用：{}",
                    format_percent(s.cpu.usage_percent as f64)
                ));
                if !s.cpu.model.is_empty() {
                    lines.push(format!("型号：{}", s.cpu.model));
                }
                if s.cpu.physical_cores > 0 {
                    lines.push(format!("物理核心：{}", s.cpu.physical_cores));
                }
            }
            "CPU 占用"
        }
        "cpu-temp" => {
            let cpu = hw.and_then(|h| h.cpu.as_ref());
            lines.push(format!(
                "封装温度：{}",
                format_temp(cpu.and_then(|c| c.package_temp_c))
            ));
            if let Some(cpu) = cpu {
                let temps: Vec<f64> = cpu.per_core_temps_c.iter().flatten().copied().collect();
                if let Some(max) = temps.iter().copied().reduce(f64::max) {
                    lines.push(format!("最高核心：{}", format_temp(Some(max))));
                }
                if let Some(min) = temps.iter().copied().reduce(f64::min) {
                    lines.push(format!("最低核心：{}", format_temp(Some(min))));
                }
            }
            "CPU 温度"
        }
        "cpu-freq" => {
            lines.push(format!(
                "当前：{}",
                format_frequency(
                    hw.and_then(|h| h.cpu.as_ref())
                        .and_then(|cpu| cpu.frequency_mhz)
                )
            ));
            "CPU 频率"
        }
        "mem" => {
            if let Some(memory) = hw.and_then(|h| h.memory.as_ref()) {
                lines.push(format!("占用：{}", format_percent(memory.used_percent)));
                lines.push(format!(
                    "已用：{} / {}",
                    format_bytes(memory.used_bytes as f64),
                    format_bytes(memory.total_bytes as f64)
                ));
                if memory.swap_total_bytes > 0 {
                    lines.push(format!(
                        "交换：{} / {}",
                        format_bytes(memory.swap_used_bytes as f64),
                        format_bytes(memory.swap_total_bytes as f64)
                    ));
                }
            } else if let Some(s) = snapshot {
                lines.push(format!(
                    "占用：{}",
                    format_percent(s.memory.used_percent as f64)
                ));
                lines.push(format!(
                    "已用：{} / {}",
                    format_bytes(s.memory.used_bytes as f64),
                    format_bytes(s.memory.total_bytes as f64)
                ));
            }
            "内存"
        }
        "gpu-usage" => {
            append_gpu_lines(&mut lines, hw, |gpu| format_percent_opt(gpu.usage_percent));
            "GPU 占用"
        }
        "gpu-temp" => {
            append_gpu_lines(&mut lines, hw, |gpu| format_temp(gpu.temp_c));
            "GPU 温度"
        }
        "disk-read" | "disk-write" => {
            let disks = hw.map(|h| h.disks.as_slice()).unwrap_or_default();
            if disks.is_empty() {
                lines.push("未检测到磁盘".into());
            } else {
                let total_read: f64 = disks.iter().filter_map(|d| d.read_bytes_per_sec).sum();
                let total_write: f64 = disks.iter().filter_map(|d| d.write_bytes_per_sec).sum();
                lines.push(format!("合计读取：{}", format_speed(total_read)));
                lines.push(format!("合计写入：{}", format_speed(total_write)));
                for disk in disks {
                    let name = non_empty(&disk.model, "磁盘");
                    lines.push(format!(
                        "{name}：↓{} ↑{}",
                        format_speed_opt(disk.read_bytes_per_sec),
                        format_speed_opt(disk.write_bytes_per_sec)
                    ));
                }
            }
            "硬盘读写"
        }
        "disk-temp" => {
            let disks = hw.map(|h| h.disks.as_slice()).unwrap_or_default();
            if disks.is_empty() {
                lines.push("未检测到磁盘".into());
            }
            for disk in disks {
                lines.push(format!(
                    "{}：{}",
                    non_empty(&disk.model, "磁盘"),
                    format_temp(disk.temp_c)
                ));
            }
            "磁盘温度"
        }
        "fan-rpm" => {
            let fans: Vec<_> = hw
                .map(|h| {
                    h.fans
                        .iter()
                        .filter(|fan| fan.rpm.unwrap_or(0.0) > 0.0)
                        .collect()
                })
                .unwrap_or_default();
            if fans.is_empty() {
                lines.push("未检测到风扇".into());
            }
            for fan in fans {
                let pwm = fan
                    .pwm_percent
                    .map(|value| format!("（{value:.0}%）"))
                    .unwrap_or_default();
                lines.push(format!(
                    "{}：{}{pwm}",
                    non_empty(&fan.name, "风扇"),
                    fan.rpm
                        .map(|value| format!("{value:.0} RPM"))
                        .unwrap_or_else(|| "--".into())
                ));
            }
            "风扇转速"
        }
        "mb-temp" => {
            if let Some(board) = hw.and_then(|h| h.motherboard.as_ref()) {
                let board_name = format!("{} {}", board.vendor, board.model)
                    .trim()
                    .to_string();
                if !board_name.is_empty() {
                    lines.push(format!("主板：{board_name}"));
                }
                for temp in &board.temperatures_c {
                    lines.push(format!("{}：{}", temp.name, format_temp(Some(temp.value))));
                }
            }
            if lines.is_empty() {
                lines.push("未检测到温度".into());
            }
            "主板温度"
        }
        "net-down" | "net-up" => {
            if let Some(s) = snapshot {
                lines.push(format!(
                    "下行：{}",
                    format_speed(s.network.total.bytes_recv_per_sec as f64)
                ));
                lines.push(format!(
                    "上行：{}",
                    format_speed(s.network.total.bytes_sent_per_sec as f64)
                ));
            }
            "网速"
        }
        _ => return None,
    };

    if lines.is_empty() {
        lines.push("暂无数据".into());
    }
    Some(format!("{title}\r\n{}", lines.join("\r\n")))
}

#[cfg(windows)]
fn append_gpu_lines(
    lines: &mut Vec<String>,
    hw: Option<&crate::hw::HwSnapshot>,
    format_value: impl Fn(&crate::hw::GpuHw) -> String,
) {
    let gpus = hw.map(|h| h.gpus.as_slice()).unwrap_or_default();
    if gpus.is_empty() {
        lines.push("未检测到 GPU".into());
    }
    for gpu in gpus {
        let fallback = non_empty(&gpu.vendor, "GPU");
        lines.push(format!(
            "{}：{}",
            non_empty(&gpu.name, fallback),
            format_value(gpu)
        ));
    }
}

#[cfg(windows)]
fn non_empty<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value
    }
}

#[cfg(windows)]
fn format_percent(value: f64) -> String {
    format!("{value:.0}%")
}

#[cfg(windows)]
fn format_percent_opt(value: Option<f64>) -> String {
    value.map(format_percent).unwrap_or_else(|| "--".into())
}

#[cfg(windows)]
fn format_temp(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.0}°C"))
        .unwrap_or_else(|| "--".into())
}

#[cfg(windows)]
fn format_frequency(value: Option<f64>) -> String {
    match value {
        Some(value) if value >= 1000.0 => format!("{:.2} GHz", value / 1000.0),
        Some(value) if value > 0.0 => format!("{value:.0} MHz"),
        _ => "--".into(),
    }
}

#[cfg(windows)]
fn format_bytes(value: f64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    if value >= GB {
        format!("{:.2} GB", value / GB)
    } else if value >= MB {
        format!("{:.2} MB", value / MB)
    } else if value >= KB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{value:.0} B")
    }
}

#[cfg(windows)]
fn format_speed(value: f64) -> String {
    format!("{}/s", format_bytes(value))
}

#[cfg(windows)]
fn format_speed_opt(value: Option<f64>) -> String {
    value.map(format_speed).unwrap_or_else(|| "--".into())
}

#[cfg(all(test, windows))]
mod tooltip_tests {
    use super::*;

    #[test]
    fn builds_network_tooltip_from_cached_snapshot() {
        let mut snapshot = crate::monitor::Snapshot::default();
        snapshot.network.total.bytes_recv_per_sec = 2 * 1024 * 1024;
        snapshot.network.total.bytes_sent_per_sec = 512 * 1024;

        let text = build_overlay_tooltip("net-down", Some(&snapshot), None).unwrap();
        assert!(text.contains("网速"));
        assert!(text.contains("下行：2.00 MB/s"));
        assert!(text.contains("上行：512.0 KB/s"));
    }

    #[test]
    fn rejects_unknown_overlay_item() {
        assert!(build_overlay_tooltip("unknown", None, None).is_none());
    }

    #[test]
    fn summary_combines_configured_items_and_deduplicates_pairs() {
        use crate::config::OverlayItem;

        let mut snapshot = crate::monitor::Snapshot::default();
        snapshot.cpu.usage_percent = 17.0;
        snapshot.memory.used_percent = 56.0;
        snapshot.network.total.bytes_recv_per_sec = 1700;
        snapshot.network.total.bytes_sent_per_sec = 1300;
        let items = [
            OverlayItem::NetDown,
            OverlayItem::NetUp,
            OverlayItem::Cpu,
            OverlayItem::Mem,
        ];

        let text = build_overlay_summary(&items, Some(&snapshot), None).unwrap();
        assert_eq!(text.matches("网速：").count(), 1);
        assert!(text.contains("CPU 占用：17%"));
        assert!(text.contains("内存：56%"));
    }
}

#[tauri::command]
#[specta::specta]
pub fn show_config_window(app: AppHandle) -> Result<(), IpcError> {
    if let Some(w) = app.get_webview_window(CONFIG_LABEL) {
        w.show().map_err(AppError::Tauri)?;
        w.unminimize().ok();
        w.set_focus().map_err(AppError::Tauri)?;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn hide_config_window(app: AppHandle) -> Result<(), IpcError> {
    if let Some(w) = app.get_webview_window(CONFIG_LABEL) {
        w.hide().map_err(AppError::Tauri)?;
    }
    Ok(())
}

pub fn apply_overlay_config(app: &AppHandle, cfg: &OverlayConfig) -> Result<(), AppError> {
    let _ = cfg;
    if let Some(w) = app.get_webview_window(OVERLAY_LABEL) {
        w.set_always_on_top(false).map_err(AppError::Tauri)?;
        w.set_ignore_cursor_events(false).map_err(AppError::Tauri)?;
        w.show().map_err(AppError::Tauri)?;
        let _ = dock_overlay_now(app);
    }
    Ok(())
}

#[derive(serde::Deserialize, specta::Type)]
pub struct ResizeArgs {
    pub width: u32,
    pub height: u32,
}

#[tauri::command]
#[specta::specta]
pub fn resize_overlay(
    app: AppHandle,
    state: State<'_, AppState>,
    args: ResizeArgs,
) -> Result<(), IpcError> {
    let overlay_cfg = state.config.snapshot().overlay;
    if let Some(w) = app.get_webview_window(OVERLAY_LABEL) {
        let scale = w.scale_factor().unwrap_or(1.0).max(0.1);
        let mut width = logical_to_physical(args.width.max(40), scale);
        let mut height = logical_to_physical(args.height.max(20), scale);
        #[cfg(windows)]
        {
            if let Some(layout) = crate::windows_api::taskbar::find_taskbar_layout() {
                let (dock_w, dock_h) =
                    crate::windows_api::taskbar::dock_size(&layout, width as i32, height as i32);
                width = dock_w as u32;
                height = dock_h as u32;
            }
        }

        w.set_size(PhysicalSize::new(width, height))
            .map_err(AppError::Tauri)?;
    }
    // Keep the overlay flush against the taskbar after content reflow.
    let _ = overlay_cfg;
    let _ = dock_overlay_now(&app);
    Ok(())
}

fn logical_to_physical(value: u32, scale_factor: f64) -> u32 {
    ((value as f64) * scale_factor).ceil().max(1.0) as u32
}

#[tauri::command]
#[specta::specta]
pub fn dock_overlay_to_taskbar(app: AppHandle, state: State<'_, AppState>) -> Result<(), IpcError> {
    let _ = state;
    dock_overlay_now(&app)?;
    Ok(())
}
/// Reposition the overlay window to sit flush against the taskbar's
/// notification area.
pub fn dock_overlay_now(app: &AppHandle) -> Result<(), AppError> {
    #[cfg(windows)]
    {
        let layout = crate::windows_api::taskbar::find_taskbar_layout()
            .ok_or_else(|| AppError::NotFound("Shell_TrayWnd".into()))?;
        if let Some(w) = app.get_webview_window(OVERLAY_LABEL) {
            let size = w.outer_size().map_err(AppError::Tauri)?;
            let (win_w, win_h) = crate::windows_api::taskbar::dock_size(
                &layout,
                size.width as i32,
                size.height as i32,
            );
            if win_w as u32 != size.width || win_h as u32 != size.height {
                w.set_size(PhysicalSize::new(win_w as u32, win_h as u32))
                    .map_err(AppError::Tauri)?;
            }
            let (x, y) = crate::windows_api::taskbar::dock_position(&layout, win_w, win_h);
            if let Some(rect) = current_window_rect(&w) {
                let current_w = rect.right - rect.left;
                let current_h = rect.bottom - rect.top;
                if rect.left == x
                    && rect.top == y
                    && current_w == win_w
                    && current_h == win_h
                    && overlay_is_taskbar_child(&w, layout.hwnd)
                {
                    return Ok(());
                }
            }
            match dock_overlay_as_taskbar_child(&w, &layout, x, y, win_w, win_h) {
                Ok(()) => {}
                Err(e) => {
                    tracing::warn!(?e, "taskbar child dock failed; falling back to topmost");
                    restore_overlay_popup_style(&w).ok();
                    w.set_position(PhysicalPosition::new(x, y))
                        .map_err(AppError::Tauri)?;
                    force_overlay_topmost(&w)?;
                }
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = app;
    }
    Ok(())
}

#[cfg(windows)]
fn current_window_rect(w: &tauri::WebviewWindow) -> Option<RECT> {
    use windows::Win32::Foundation::HWND;

    let hwnd = HWND(w.hwnd().ok()?.0 as _);
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect).ok()? };
    Some(rect)
}

#[cfg(windows)]
fn overlay_is_taskbar_child(
    w: &tauri::WebviewWindow,
    taskbar: windows::Win32::Foundation::HWND,
) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::GetParent;

    let Ok(hwnd) = w.hwnd() else {
        return false;
    };
    unsafe { GetParent(HWND(hwnd.0 as _)).ok() == Some(taskbar) }
}

pub fn spawn_taskbar_overlay_z_order_watchdog(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(e) = restore_taskbar_overlay(&app) {
                tracing::debug!(?e, "taskbar overlay z-order watchdog skipped");
            }
        }
    });
}

fn restore_taskbar_overlay(app: &AppHandle) -> Result<(), AppError> {
    if let Some(w) = get_or_create_overlay_window(app)? {
        w.set_always_on_top(false).ok();
        w.set_ignore_cursor_events(false).ok();
        if !w.is_visible().unwrap_or(false) {
            w.show().map_err(AppError::Tauri)?;
        }
        dock_overlay_now(app)?;
    }
    Ok(())
}

fn get_or_create_overlay_window(app: &AppHandle) -> Result<Option<tauri::WebviewWindow>, AppError> {
    if let Some(w) = app.get_webview_window(OVERLAY_LABEL) {
        return Ok(Some(w));
    }

    let Some(config) = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == OVERLAY_LABEL)
    else {
        return Ok(None);
    };

    let w = tauri::WebviewWindowBuilder::from_config(app, config)
        .map_err(AppError::Tauri)?
        .build()
        .map_err(AppError::Tauri)?;
    Ok(Some(w))
}

#[cfg(windows)]
fn force_overlay_topmost(w: &tauri::WebviewWindow) -> Result<(), AppError> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };

    let hwnd = HWND(w.hwnd().map_err(AppError::Tauri)?.0 as _);
    unsafe {
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
        .map_err(AppError::Windows)?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn force_overlay_topmost(_w: &tauri::WebviewWindow) -> Result<(), AppError> {
    Ok(())
}

#[cfg(windows)]
fn dock_overlay_as_taskbar_child(
    w: &tauri::WebviewWindow,
    layout: &crate::windows_api::taskbar::TaskbarLayout,
    screen_x: i32,
    screen_y: i32,
    width: i32,
    height: i32,
) -> Result<(), AppError> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetParent, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE,
        HWND_TOP, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_SHOWWINDOW, WS_CHILD, WS_CLIPSIBLINGS,
        WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
    };

    let hwnd = HWND(w.hwnd().map_err(AppError::Tauri)?.0 as _);
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let wanted_style = (style | WS_CHILD.0 | WS_CLIPSIBLINGS.0) & !WS_POPUP.0;
        if wanted_style != style {
            SetWindowLongPtrW(hwnd, GWL_STYLE, wanted_style as isize);
        }

        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let wanted_ex_style =
            (ex_style | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) & !WS_EX_APPWINDOW.0;
        if wanted_ex_style != ex_style {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, wanted_ex_style as isize);
        }

        SetParent(hwnd, layout.hwnd).map_err(AppError::Windows)?;
        let x = screen_x - layout.bar.x;
        let y = screen_y - layout.bar.y;
        SetWindowPos(
            hwnd,
            HWND_TOP,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW,
        )
        .map_err(AppError::Windows)?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn dock_overlay_as_taskbar_child(
    _w: &tauri::WebviewWindow,
    _layout: &crate::windows_api::taskbar::TaskbarLayout,
    _screen_x: i32,
    _screen_y: i32,
    _width: i32,
    _height: i32,
) -> Result<(), AppError> {
    Ok(())
}

#[cfg(windows)]
fn restore_overlay_popup_style(w: &tauri::WebviewWindow) -> Result<(), AppError> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetParent, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE,
        HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WS_CHILD,
        WS_EX_NOACTIVATE, WS_POPUP,
    };

    let hwnd = HWND(w.hwnd().map_err(AppError::Tauri)?.0 as _);
    unsafe {
        SetParent(hwnd, HWND::default()).map_err(AppError::Windows)?;

        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let wanted_style = (style | WS_POPUP.0) & !WS_CHILD.0;
        if wanted_style != style {
            SetWindowLongPtrW(hwnd, GWL_STYLE, wanted_style as isize);
        }

        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let wanted_ex_style = ex_style & !WS_EX_NOACTIVATE.0;
        if wanted_ex_style != ex_style {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, wanted_ex_style as isize);
        }

        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
        .map_err(AppError::Windows)?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn restore_overlay_popup_style(_w: &tauri::WebviewWindow) -> Result<(), AppError> {
    Ok(())
}
