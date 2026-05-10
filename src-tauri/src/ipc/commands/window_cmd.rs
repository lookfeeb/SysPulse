use crate::app::AppState;
use crate::config::OverlayConfig;
use crate::error::{AppError, IpcError};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, State};

const CONFIG_LABEL: &str = "config";
const OVERLAY_LABEL: &str = "overlay";
const OVERLAY_TOOLTIP_LABEL: &str = "overlay-tooltip";

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

pub fn spawn_taskbar_overlay_z_order_watchdog(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(1500));
        loop {
            interval.tick().await;
            if let Err(e) = restore_taskbar_overlay_if_config_visible(&app) {
                tracing::debug!(?e, "taskbar overlay z-order watchdog skipped");
            }
        }
    });
}

fn restore_taskbar_overlay_if_config_visible(app: &AppHandle) -> Result<(), AppError> {
    let Some(state) = app.try_state::<AppState>() else {
        return Ok(());
    };
    let cfg = state.config.snapshot().overlay;
    let _ = cfg;

    if let Some(w) = app.get_webview_window(OVERLAY_LABEL) {
        w.set_always_on_top(false).ok();
        let is_window_visible = w.is_visible().unwrap_or(false);
        if is_window_visible {
            dock_overlay_now(app)?;
        } else {
            w.show().map_err(AppError::Tauri)?;
            dock_overlay_now(app)?;
        }
    }
    Ok(())
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


#[derive(serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ShowTooltipArgs {
    pub title: String,
    pub lines: Vec<String>,
    /// Anchor x in physical pixels (usually the hovered item's screen x).
    pub anchor_x: i32,
    /// Anchor y in physical pixels (usually the overlay top in screen coords).
    pub anchor_y: i32,
}

#[derive(serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OverlayTooltipFitArgs {
    pub width: u32,
    pub height: u32,
}

#[tauri::command]
#[specta::specta]
pub fn show_overlay_tooltip(app: AppHandle, args: ShowTooltipArgs) -> Result<(), IpcError> {
    if let Some(w) = app.get_webview_window(OVERLAY_TOOLTIP_LABEL) {
        // Push payload to the tooltip webview to render.
        use tauri::Emitter;
        #[derive(serde::Serialize, Clone)]
        struct Payload {
            title: String,
            lines: Vec<String>,
        }
        let _ = app.emit_to(
            OVERLAY_TOOLTIP_LABEL,
            "overlay-tooltip:show",
            Payload {
                title: args.title,
                lines: args.lines,
            },
        );

        // Provisional position (the tooltip webview will call back with its real size).
        w.set_position(PhysicalPosition::new(args.anchor_x, args.anchor_y))
            .map_err(AppError::Tauri)?;
        w.set_always_on_top(true).map_err(AppError::Tauri)?;
        w.show().map_err(AppError::Tauri)?;
        // Remember the anchor so we can reflow after size is known.
        if let Some(state) = app.try_state::<AppState>() {
            *state.last_overlay_tooltip_anchor.write() = Some((args.anchor_x, args.anchor_y));
        }
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn hide_overlay_tooltip(app: AppHandle) -> Result<(), IpcError> {
    if let Some(w) = app.get_webview_window(OVERLAY_TOOLTIP_LABEL) {
        w.hide().map_err(AppError::Tauri)?;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn overlay_tooltip_fit(
    app: AppHandle,
    state: State<'_, AppState>,
    args: OverlayTooltipFitArgs,
) -> Result<(), IpcError> {
    if let Some(w) = app.get_webview_window(OVERLAY_TOOLTIP_LABEL) {
        let scale = w.scale_factor().unwrap_or(1.0).max(0.1);
        let width = ((args.width as f64) * scale).ceil().max(40.0) as u32;
        let height = ((args.height as f64) * scale).ceil().max(20.0) as u32;
        w.set_size(PhysicalSize::new(width, height))
            .map_err(AppError::Tauri)?;

        if let Some((anchor_x, anchor_y)) = *state.last_overlay_tooltip_anchor.read() {
            let (x, y) = clamp_tooltip_position(
                anchor_x,
                anchor_y,
                width as i32,
                height as i32,
            );
            w.set_position(PhysicalPosition::new(x, y))
                .map_err(AppError::Tauri)?;
        }
    }
    Ok(())
}

/// Choose a tooltip position that prefers "above the anchor" (since the overlay
/// sits on the taskbar). Falls back to below if there's no room.
fn clamp_tooltip_position(anchor_x: i32, anchor_y: i32, width: i32, height: i32) -> (i32, i32) {
    // Best effort: clamp against the primary monitor if we can query it.
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        };

        let pt = POINT {
            x: anchor_x,
            y: anchor_y,
        };
        let hmon = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let ok = unsafe { GetMonitorInfoW(hmon, &mut info).as_bool() };
        if ok {
            let work = info.rcWork;
            let gap = 6;
            // Prefer above anchor
            let above_y = anchor_y - height - gap;
            let below_y = anchor_y + gap;
            let y = if above_y >= work.top {
                above_y
            } else {
                below_y.min(work.bottom - height)
            };
            let mut x = anchor_x - width / 2;
            if x < work.left + 4 {
                x = work.left + 4;
            }
            if x + width > work.right - 4 {
                x = work.right - width - 4;
            }
            return (x, y);
        }
    }
    (anchor_x, anchor_y - height - 6)
}
