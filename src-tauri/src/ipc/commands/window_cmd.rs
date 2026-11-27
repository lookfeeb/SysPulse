use crate::app::AppState;
use crate::config::OverlayConfig;
use crate::error::{AppError, IpcError};
use tauri::{AppHandle, Manager, State};
#[cfg(windows)]
use windows::Win32::Foundation::RECT;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

const CONFIG_LABEL: &str = "config";
const OVERLAY_LABEL: &str = "overlay";

#[cfg(all(test, windows, feature = "desktop-tests"))]
#[path = "window_cmd/desktop_tests.rs"]
mod desktop_tests;

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
        let mut pending_key: Option<String> = None;
        let mut hover_started = Instant::now();
        let mut last_refresh = Instant::now() - Duration::from_secs(2);

        loop {
            interval.tick().await;
            if crate::app::is_shutting_down() {
                let _ = app.run_on_main_thread(crate::windows_api::custom_tooltip::destroy);
                break;
            }
            let hovered = overlay_tooltip_hit_test(&app);
            let next_key = hovered.as_ref().map(|hit| hit.key.as_str());
            let changed = next_key != active_key.as_deref();

            let Some(hit) = hovered else {
                pending_key = None;
                if active_key.take().is_some() {
                    let _ = app.run_on_main_thread(crate::windows_api::custom_tooltip::hide);
                }
                continue;
            };

            if pending_key.as_deref() != Some(hit.key.as_str()) {
                pending_key = Some(hit.key.clone());
                hover_started = Instant::now();
            }
            if changed && hover_started.elapsed() < Duration::from_millis(240) {
                continue;
            }

            if !changed && last_refresh.elapsed() < Duration::from_secs(1) {
                continue;
            }
            let Some(state) = app.try_state::<AppState>() else {
                continue;
            };
            let config = state.config.snapshot();
            let show_traffic = matches!(hit.key.as_str(), "net-down" | "net-up")
                || (hit.key == "__all__"
                    && config.overlay.items.iter().any(|item| {
                        matches!(
                            item,
                            crate::config::OverlayItem::NetDown | crate::config::OverlayItem::NetUp
                        )
                    }));
            let today_traffic = if show_traffic {
                match query_today_traffic(
                    &state.writer,
                    &config.network,
                    chrono::Local::now().date_naive(),
                )
                .await
                {
                    Ok(total) => Some(total),
                    Err(error) => {
                        tracing::warn!(?error, "query tooltip daily traffic failed");
                        None
                    }
                }
            } else {
                None
            };
            // A database read yields to other tasks; don't reopen a card after
            // the pointer has left its region or the overlay has moved.
            let Some(hit) = overlay_tooltip_hit_test(&app)
                .filter(|current| current.key == hit.key && current.owner == hit.owner)
            else {
                pending_key = None;
                if active_key.take().is_some() {
                    let _ = app.run_on_main_thread(crate::windows_api::custom_tooltip::hide);
                }
                continue;
            };
            let snapshot = state.last_snapshot.read();
            let hw = state.last_hw_snapshot.read();
            let fresh_hw = hw.as_ref().filter(|s| s.is_fresh());
            let text = if hit.key == "__all__" {
                build_overlay_summary(
                    &config.overlay.items,
                    snapshot.as_ref(),
                    fresh_hw,
                    today_traffic,
                )
            } else {
                build_overlay_tooltip(&hit.key, snapshot.as_ref(), fresh_hw, today_traffic)
            };
            let Some(text) = text else {
                if active_key.take().is_some() {
                    let _ = app.run_on_main_thread(crate::windows_api::custom_tooltip::hide);
                }
                continue;
            };

            let key = hit.key.clone();
            let owner_raw = hit.owner;
            let anchor = hit.anchor;
            let _ = app.run_on_main_thread(move || {
                let owner = HWND(owner_raw as *mut _);
                if let Err(error) = crate::windows_api::custom_tooltip::show(owner, &text, anchor) {
                    tracing::warn!(?error, key = %key, "show custom overlay tooltip failed");
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
async fn query_today_traffic(
    writer: &crate::storage::WriterHandle,
    network: &crate::config::schema::NetworkConfig,
    today: chrono::NaiveDate,
) -> crate::error::Result<(u64, u64)> {
    use crate::config::schema::NetworkMonitorMode;
    use crate::storage::queries::{HistoryGranularity, HistoryQuery};

    let interfaces = match network.monitor_mode {
        NetworkMonitorMode::All => vec![None],
        NetworkMonitorMode::Specified => network
            .monitor_interfaces
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|iface| Some(iface.clone()))
            .collect(),
    };
    let date = today.format("%Y-%m-%d").to_string();
    let mut total = (0_u64, 0_u64);
    for iface in interfaces {
        // The writer combines persisted history and queued samples exactly once.
        // Only query while the network card is visible; never force a disk flush.
        let rows = writer
            .query_history(HistoryQuery {
                from: Some(date.clone()),
                to: Some(date.clone()),
                granularity: HistoryGranularity::Day,
                iface,
            })
            .await?;
        for row in rows {
            total.0 = total.0.saturating_add(row.bytes_recv.max(0) as u64);
            total.1 = total.1.saturating_add(row.bytes_sent.max(0) as u64);
        }
    }
    Ok(total)
}

#[cfg(windows)]
fn today_traffic_line(total: Option<(u64, u64)>) -> String {
    let (recv, sent) = total
        .map(|(recv, sent)| (format_bytes(recv as f64), format_bytes(sent as f64)))
        .unwrap_or_else(|| ("--".into(), "--".into()));
    format!("今日流量：↓{recv}  ↑{sent}")
}

#[cfg(windows)]
struct OverlayTooltipHit {
    key: String,
    owner: isize,
    anchor: RECT,
}

#[cfg(windows)]
fn overlay_tooltip_hit_test(app: &AppHandle) -> Option<OverlayTooltipHit> {
    use windows::Win32::Foundation::{HWND, POINT};
    use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, WindowFromPoint};

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
        // A menu or another window covering the overlay must dismiss the card.
        if !crate::windows_api::taskbar::is_overlay_hit(owner, WindowFromPoint(cursor)) {
            return None;
        }
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
        anchor: RECT {
            left: window_rect.left + (region.left * scale).round() as i32,
            top: window_rect.top + (region.top * scale).round() as i32,
            right: window_rect.left + (region.right * scale).round() as i32,
            bottom: window_rect.top + (region.bottom * scale).round() as i32,
        },
    })
}

#[cfg(windows)]
fn build_overlay_summary(
    items: &[crate::config::OverlayItem],
    snapshot: Option<&crate::monitor::Snapshot>,
    hw: Option<&crate::hw::HwSnapshot>,
    today_traffic: Option<(u64, u64)>,
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
                    "网速：↓{}  ↑{}\r\n{}",
                    total
                        .map(|value| format_speed(value.bytes_recv_per_sec as f64))
                        .unwrap_or_else(|| "--".into()),
                    total
                        .map(|value| format_speed(value.bytes_sent_per_sec as f64))
                        .unwrap_or_else(|| "--".into()),
                    today_traffic_line(today_traffic)
                ))
            }
            OverlayItem::Cpu => Some(format!(
                "CPU：{}",
                hw.and_then(|h| h.cpu.as_ref())
                    .and_then(|cpu| cpu.total_usage)
                    .map(format_percent)
                    .or_else(|| snapshot.map(|s| format_percent(s.cpu.usage_percent as f64)))
                    .unwrap_or_else(|| "--".into())
            )),
            OverlayItem::CpuTemp => Some(format!(
                "CPU°：{}",
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
                        "内存：{percent} ({} / {})",
                        format_bytes(value.used_bytes as f64),
                        format_bytes(value.total_bytes as f64)
                    ),
                    _ => format!("内存：{percent}"),
                })
            }
            OverlayItem::Gpu | OverlayItem::GpuUsage => Some(format!(
                "GPU：{}",
                format_percent_opt(max_value(
                    hw.into_iter()
                        .flat_map(|h| { h.gpus.iter().filter_map(|gpu| gpu.usage_percent) })
                ))
            )),
            OverlayItem::GpuTemp => Some(format!(
                "GPU°：{}",
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
                    "硬盘：读 {} · 写 {}",
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
                        .filter(|rpm| *rpm >= 0.0)
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
    today_traffic: Option<(u64, u64)>,
) -> Option<String> {
    let mut lines = Vec::new();
    let title = match key {
        "cpu" => {
            if let Some(cpu) = hw.and_then(|h| h.cpu.as_ref()) {
                lines.push(format!(
                    "当前占用：{}",
                    cpu.total_usage
                        .map(format_percent)
                        .unwrap_or_else(|| "--".into())
                ));
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
            "CPU"
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
            "CPU°"
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
            "GPU"
        }
        "gpu-temp" => {
            append_gpu_lines(&mut lines, hw, |gpu| format_temp(gpu.temp_c));
            "GPU°"
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
            lines.push(today_traffic_line(today_traffic));
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

        for key in ["net-down", "net-up"] {
            let text = build_overlay_tooltip(
                key,
                Some(&snapshot),
                None,
                Some((3 * 1024 * 1024 * 1024, 512 * 1024 * 1024)),
            )
            .unwrap();
            assert!(text.contains("网速"));
            assert!(text.contains("下行：2.00 MB/s"));
            assert!(text.contains("上行：512.0 KB/s"));
            assert!(text.contains("今日流量：↓3.00 GB  ↑512.00 MB"));
        }
    }

    #[test]
    fn rejects_unknown_overlay_item() {
        assert!(build_overlay_tooltip("unknown", None, None, None).is_none());
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

        let text = build_overlay_summary(&items, Some(&snapshot), None, Some((0, 0))).unwrap();
        assert_eq!(text.matches("网速：").count(), 1);
        assert_eq!(text.matches("今日流量：").count(), 1);
        assert_eq!(text.lines().nth(2), Some("今日流量：↓0 B  ↑0 B"));
        assert!(text.contains("CPU：17%"));
        assert!(text.contains("内存：56%"));
        let text = build_overlay_summary(&[OverlayItem::Cpu], Some(&snapshot), None, None).unwrap();
        assert!(!text.contains("今日流量"));
        let text = build_overlay_summary(&[OverlayItem::NetUp], None, None, None).unwrap();
        assert!(text.contains("今日流量：↓--  ↑--"));
    }

    #[tokio::test]
    async fn daily_tooltip_includes_saved_and_live_bytes_and_follows_date_and_interfaces() {
        use crate::config::schema::{NetworkConfig, NetworkMonitorMode};
        use crate::storage::{spawn_writer, TrafficDelta};
        use chrono::NaiveDate;
        use std::sync::Arc;

        let pool = Arc::new(
            r2d2::Pool::builder()
                .max_size(1)
                .build(r2d2_sqlite::SqliteConnectionManager::memory())
                .unwrap(),
        );
        pool.get().unwrap().execute_batch(
            "CREATE TABLE traffic_daily(date_iso TEXT, luid INTEGER, bytes_recv INTEGER, bytes_sent INTEGER, PRIMARY KEY(date_iso,luid));
             INSERT INTO traffic_daily VALUES ('2026-11-07',1,900,800), ('2026-11-08',1,100,200), ('2026-11-08',2,300,400);"
        ).unwrap();
        let writer = spawn_writer(pool);
        let today = NaiveDate::from_ymd_opt(2026, 11, 8).unwrap();
        let tomorrow = today.succ_opt().unwrap();
        let mut network = NetworkConfig::default();
        assert_eq!(
            query_today_traffic(&writer, &network, today).await.unwrap(),
            (400, 600)
        );
        writer
            .send_delta(TrafficDelta {
                luid: 1,
                date_local: today,
                bytes_recv: 10,
                bytes_sent: 20,
            })
            .await;
        for _ in 0..2 {
            assert_eq!(
                query_today_traffic(&writer, &network, today).await.unwrap(),
                (410, 620)
            );
            writer.flush().await.unwrap();
        }
        network.monitor_mode = NetworkMonitorMode::Specified;
        network.monitor_interfaces = vec!["1".into(), "1".into()];
        assert_eq!(
            query_today_traffic(&writer, &network, today).await.unwrap(),
            (110, 220)
        );
        network.monitor_interfaces = vec!["2".into(), "1".into()];
        assert_eq!(
            query_today_traffic(&writer, &network, today).await.unwrap(),
            (410, 620)
        );
        network.monitor_interfaces.clear();
        assert_eq!(
            query_today_traffic(&writer, &network, today).await.unwrap(),
            (0, 0)
        );
        network.monitor_mode = NetworkMonitorMode::All;
        assert_eq!(
            query_today_traffic(&writer, &network, tomorrow)
                .await
                .unwrap(),
            (0, 0)
        );
        writer
            .send_delta(TrafficDelta {
                luid: 1,
                date_local: tomorrow,
                bytes_recv: 5,
                bytes_sent: 6,
            })
            .await;
        assert_eq!(
            query_today_traffic(&writer, &network, tomorrow)
                .await
                .unwrap(),
            (5, 6)
        );
        assert_eq!(
            query_today_traffic(&writer, &network, today).await.unwrap(),
            (410, 620)
        );
    }
}

#[tauri::command]
#[specta::specta]
pub fn show_config_window(app: AppHandle) -> Result<(), IpcError> {
    open_config_window(&app).map_err(Into::into)
}

pub(crate) fn open_config_window(app: &AppHandle) -> Result<(), AppError> {
    let w = app
        .get_webview_window(CONFIG_LABEL)
        .ok_or_else(|| AppError::NotFound(CONFIG_LABEL.into()))?;
    w.show().map_err(AppError::Tauri)?;
    w.unminimize().map_err(AppError::Tauri)?;
    w.set_focus().map_err(AppError::Tauri)?;
    tracing::debug!("config window shown");
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
        w.set_ignore_cursor_events(false).map_err(AppError::Tauri)?;
        #[cfg(not(windows))]
        {
            w.set_always_on_top(true).map_err(AppError::Tauri)?;
            w.show().map_err(AppError::Tauri)?;
        }
        watch_overlay_events(app, &w)?;
        dock_overlay_now(app)?;
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
    let _ = state;
    if let Some(w) = app.get_webview_window(OVERLAY_LABEL) {
        let scale = w.scale_factor().unwrap_or(1.0).max(0.1);
        let width = logical_to_physical(args.width.max(40), scale);
        let height = logical_to_physical(args.height.max(20), scale);
        #[cfg(windows)]
        dock_overlay_with_size(&app, Some((width as i32, height as i32)))?;
        #[cfg(not(windows))]
        w.set_size(tauri::PhysicalSize::new(width, height))
            .map_err(AppError::Tauri)?;
    }
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
    dock_overlay_with_size(app, None)
}

fn dock_overlay_with_size(
    app: &AppHandle,
    content_size: Option<(i32, i32)>,
) -> Result<(), AppError> {
    #[cfg(windows)]
    {
        let w = app
            .get_webview_window(OVERLAY_LABEL)
            .ok_or_else(|| AppError::NotFound(OVERLAY_LABEL.into()))?;
        let hwnd = w.hwnd()?.0 as isize;
        app.run_on_main_thread(move || {
            use windows::Win32::Foundation::HWND;
            use windows::Win32::UI::WindowsAndMessaging::IsWindow;

            let hwnd = HWND(hwnd as *mut _);
            if crate::app::is_shutting_down() || !unsafe { IsWindow(hwnd).as_bool() } {
                return;
            }
            if let Err(error) = crate::windows_api::taskbar::dock_overlay(hwnd, content_size) {
                tracing::warn!(?error, "dock taskbar overlay failed");
            }
        })?;
    }
    #[cfg(not(windows))]
    let _ = (app, content_size);
    Ok(())
}

pub fn spawn_taskbar_overlay_watchdog(app: AppHandle) {
    #[cfg(windows)]
    let mut recovery = crate::windows_api::taskbar::OverlayRecovery::new(
        crate::windows_api::taskbar::ShellIdentity::current(),
    );
    #[cfg(windows)]
    if let Some(window) = app.get_webview_window(CONFIG_LABEL) {
        let listener = window.hwnd().map(|hwnd| hwnd.0 as isize);
        if let Ok(hwnd) = listener {
            let _ = app.run_on_main_thread(move || {
                if let Err(error) = crate::windows_api::taskbar::watch_taskbar_restarts(
                    windows::Win32::Foundation::HWND(hwnd as *mut _),
                ) {
                    tracing::warn!(?error, "taskbar restart broadcast listener unavailable");
                }
            });
        }
    }
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if crate::app::is_shutting_down() {
                break;
            }
            if let Err(e) = restore_taskbar_overlay(
                &app,
                #[cfg(windows)]
                &mut recovery,
            ) {
                tracing::debug!(?e, "taskbar overlay z-order watchdog skipped");
            }
        }
    });
}

fn restore_taskbar_overlay(
    app: &AppHandle,
    #[cfg(windows)] recovery: &mut crate::windows_api::taskbar::OverlayRecovery,
) -> Result<(), AppError> {
    #[cfg(windows)]
    {
        use crate::windows_api::taskbar::{RecoveryAction, ShellIdentity};
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::IsWindow;

        let window = app.get_webview_window(OVERLAY_LABEL);
        let valid = window.as_ref().map(|window| {
            window
                .hwnd()
                .is_ok_and(|hwnd| unsafe { IsWindow(HWND(hwnd.0 as *mut _)).as_bool() })
        });
        match recovery.observe(ShellIdentity::current(), valid) {
            RecoveryAction::Wait => return Ok(()),
            RecoveryAction::Destroy => {
                tracing::info!(
                    "taskbar restarted or overlay HWND lost; rebuilding overlay WebView"
                );
                // Destroy queues runtime cleanup. Wait until Tauri removes the
                // label before creating its replacement, avoiding duplicate labels.
                if let Some(window) = window {
                    window.destroy()?;
                    recovery.destroy_requested();
                }
                return Ok(());
            }
            RecoveryAction::DockOrCreate => {}
        }
    }
    if get_or_create_overlay_window(app)?.is_some() {
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
    tracing::info!("overlay WebView recreated");
    watch_overlay_events(app, &w)?;
    Ok(Some(w))
}

fn watch_overlay_events(app: &AppHandle, w: &tauri::WebviewWindow) -> Result<(), AppError> {
    #[cfg(windows)]
    {
        let hwnd = w.hwnd()?.0 as isize;
        let input_app = app.clone();
        app.run_on_main_thread(move || {
            if crate::app::is_shutting_down() {
                return;
            }
            if let Err(error) = crate::windows_api::taskbar::watch_overlay_z_order(
                windows::Win32::Foundation::HWND(hwnd as *mut _),
            ) {
                tracing::warn!(?error, "overlay z-order event hooks unavailable");
            }
            if let Err(error) = crate::windows_api::taskbar::watch_overlay_input(
                windows::Win32::Foundation::HWND(hwnd as *mut _),
                move || {
                    if crate::app::is_shutting_down() {
                        return;
                    }
                    tracing::info!("overlay native double-click: opening config window");
                    if let Err(error) = open_config_window(&input_app) {
                        tracing::warn!(?error, "open config from overlay double-click failed");
                    }
                },
            ) {
                tracing::warn!(?error, "overlay native double-click handler unavailable");
            }
        })?;
    }
    #[cfg(not(windows))]
    let _ = (app, w);
    Ok(())
}
