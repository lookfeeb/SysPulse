//! Helpers for locating the Windows shell taskbar (`Shell_TrayWnd`) so we can
//! position the overlay window in its notification area.
//!
//! True COM-based deskbands are deprecated on Windows 11. We instead use a
//! topmost popup window sized to the taskbar and placed precisely where the
//! deskband would have lived.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GetWindowRect, IsWindow, SetWindowPos,
    EVENT_OBJECT_REORDER, EVENT_OBJECT_SHOW, EVENT_SYSTEM_FOREGROUND, HWND_TOPMOST, OBJID_WINDOW,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WINEVENT_OUTOFCONTEXT,
};

thread_local! {
    // OUTOFCONTEXT callbacks run on the thread that installs the hooks. Keep
    // installation, updates and teardown on the Tauri message-loop thread.
    static Z_ORDER_HOOKS: std::cell::RefCell<Option<OverlayHooks>> = const { std::cell::RefCell::new(None) };
    static HANDLING_Z_ORDER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

struct OverlayHooks {
    handles: Vec<HWINEVENTHOOK>,
    overlay: HWND,
    #[cfg(test)]
    reference_override: Option<HWND>,
}

impl Drop for OverlayHooks {
    fn drop(&mut self) {
        for hook in self.handles.drain(..) {
            unsafe {
                let _ = UnhookWinEvent(hook);
            }
        }
    }
}

/// Install on the UI thread. The timer is only a recovery fallback: shell
/// activation/reordering is repaired as soon as the native event is delivered.
pub fn watch_overlay_z_order(overlay: HWND) -> windows::core::Result<()> {
    Z_ORDER_HOOKS.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(hooks) = slot.as_mut() {
            hooks.overlay = overlay;
            return Ok(());
        }
        let mut hooks = OverlayHooks {
            handles: Vec::new(),
            overlay,
            #[cfg(test)]
            reference_override: None,
        };
        for (first, last) in [
            (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
            (EVENT_OBJECT_SHOW, EVENT_OBJECT_REORDER),
        ] {
            let hook = unsafe {
                SetWinEventHook(
                    first,
                    last,
                    None,
                    Some(on_shell_window_event),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                )
            };
            if hook.0.is_null() {
                return Err(windows::core::Error::from_win32());
            }
            hooks.handles.push(hook);
        }
        *slot = Some(hooks);
        tracing::info!("overlay z-order event hooks installed");
        Ok(())
    })
}

pub fn stop_overlay_z_order_watch() {
    Z_ORDER_HOOKS.with(|slot| {
        slot.borrow_mut().take();
    });
}

unsafe extern "system" fn on_shell_window_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    _window: HWND,
    object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    // Reorder can name the desktop/client parent rather than the taskbar.
    if event != EVENT_SYSTEM_FOREGROUND && event != EVENT_OBJECT_REORDER && object != OBJID_WINDOW.0
    {
        return;
    }
    HANDLING_Z_ORDER.with(|handling| {
        if handling.replace(true) {
            return;
        }
        // Do not hold the RefCell borrow across Win32 calls: they can dispatch
        // nested events while menus run their own message loop.
        let target = Z_ORDER_HOOKS.with(|slot| {
            slot.borrow().as_ref().map(|hooks| {
                #[cfg(test)]
                let reference = hooks.reference_override;
                #[cfg(not(test))]
                let reference = None;
                (hooks.overlay, reference)
            })
        });
        if let Some((overlay, reference)) = target {
            if IsWindow(overlay).as_bool() {
                let result = match reference {
                    Some(reference) => ensure_above_window(overlay, reference).map(|_| ()),
                    None => ensure_overlay_above_taskbar(overlay),
                };
                if let Err(error) = result {
                    tracing::debug!(?error, "overlay event z-order restore skipped");
                }
            }
        }
        handling.set(false);
    });
}

/// A topmost flag alone does not preserve order among topmost windows: Explorer
/// can raise the taskbar later, and Tao ignores repeated `always_on_top(true)`.
pub fn ensure_overlay_above_taskbar(overlay: HWND) -> windows::core::Result<()> {
    let Ok(taskbar) = (unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }) else {
        return Ok(());
    };
    ensure_above_window(overlay, taskbar).map(|_| ())
}

fn ensure_above_window(overlay: HWND, reference: HWND) -> windows::core::Result<bool> {
    // Leave other topmost windows (including menus and tooltips) alone when the
    // overlay is already above the taskbar. Never activate or reparent it.
    if !is_above_window(reference, overlay) {
        return Ok(false);
    }
    unsafe {
        SetWindowPos(
            overlay,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )?;
    }
    Ok(true)
}

fn is_above_window(upper: HWND, lower: HWND) -> bool {
    struct Order {
        upper: HWND,
        lower: HWND,
        upper_first: bool,
    }
    unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> BOOL {
        let order = &mut *(data.0 as *mut Order);
        if hwnd == order.upper {
            order.upper_first = true;
            return BOOL(0);
        }
        BOOL((hwnd != order.lower) as i32)
    }
    let mut order = Order {
        upper,
        lower,
        upper_first: false,
    };
    // EnumWindows walks top-level windows in Z order and safely tolerates
    // concurrent window destruction. Stopping the callback returns an error.
    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM((&mut order as *mut Order) as isize));
    }
    order.upper_first
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskbarEdge {
    Bottom,
    Top,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy)]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct TaskbarLayout {
    pub edge: TaskbarEdge,
    /// Whole `Shell_TrayWnd` rect in physical pixels (screen coords).
    pub bar: PixelRect,
    /// `TrayNotifyWnd` rect — the right cluster (clock + system tray + chevron).
    /// May be `None` if not found (rare; degraded handling: dock to bar's far end).
    pub tray_notify: Option<PixelRect>,
}

impl TaskbarLayout {
    pub fn available_dock_size(&self) -> (i32, i32) {
        match self.edge {
            TaskbarEdge::Bottom | TaskbarEdge::Top => {
                let right_edge = self
                    .tray_notify
                    .map(|r| r.x)
                    .unwrap_or(self.bar.x + self.bar.w);
                ((right_edge - self.bar.x).max(40), self.bar.h.max(20))
            }
            TaskbarEdge::Left | TaskbarEdge::Right => {
                let bottom_edge = self
                    .tray_notify
                    .map(|r| r.y)
                    .unwrap_or(self.bar.y + self.bar.h);
                (self.bar.w.max(40), (bottom_edge - self.bar.y).max(20))
            }
        }
    }
}

pub fn find_taskbar_layout() -> Option<TaskbarLayout> {
    unsafe {
        let taskbar = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()).ok()?;
        let mut tb_rect = RECT::default();
        GetWindowRect(taskbar, &mut tb_rect).ok()?;

        let bar = PixelRect {
            x: tb_rect.left,
            y: tb_rect.top,
            w: tb_rect.right - tb_rect.left,
            h: tb_rect.bottom - tb_rect.top,
        };
        let edge = detect_edge(&tb_rect);

        let tray_notify_hwnd = FindWindowExW(
            taskbar,
            HWND::default(),
            w!("TrayNotifyWnd"),
            PCWSTR::null(),
        )
        .ok();
        let tray_notify = tray_notify_hwnd.and_then(effective_tray_notify_rect);

        Some(TaskbarLayout {
            edge,
            bar,
            tray_notify,
        })
    }
}

fn effective_tray_notify_rect(hwnd: HWND) -> Option<PixelRect> {
    unsafe {
        let tray = window_rect(hwnd)?;

        let sys_pager = FindWindowExW(hwnd, HWND::default(), w!("SysPager"), PCWSTR::null())
            .ok()
            .and_then(window_rect);
        let clock = FindWindowExW(hwnd, HWND::default(), w!("TrayClockWClass"), PCWSTR::null())
            .ok()
            .and_then(window_rect);

        let mut effective = tray;
        if let Some(left) = [sys_pager, clock]
            .into_iter()
            .flatten()
            .filter(|r| r.w > 0 && r.h > 0)
            .map(|r| r.x)
            .min()
        {
            if left > tray.x && left < tray.x + tray.w {
                effective.w = tray.x + tray.w - left;
                effective.x = left;
            }
        }
        Some(effective)
    }
}

fn window_rect(hwnd: HWND) -> Option<PixelRect> {
    unsafe {
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r).is_ok() {
            Some(PixelRect {
                x: r.left,
                y: r.top,
                w: r.right - r.left,
                h: r.bottom - r.top,
            })
        } else {
            None
        }
    }
}

fn detect_edge(rect: &RECT) -> TaskbarEdge {
    // Heuristic vs the primary monitor work area would be more correct, but the
    // taskbar's own aspect ratio gets us 99% of the way there.
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    if w >= h {
        // Horizontal taskbar — bottom is the default; if its top is at y=0, top.
        if rect.top <= 0 {
            TaskbarEdge::Top
        } else {
            TaskbarEdge::Bottom
        }
    } else {
        // Vertical taskbar
        if rect.left <= 0 {
            TaskbarEdge::Left
        } else {
            TaskbarEdge::Right
        }
    }
}

/// Compute the (x, y) at which to place a window of the given outer size so it
/// sits flush against the taskbar's notification area.
///
/// Returns `(x, y)` in screen pixels, suitable for `Window::set_position`.
pub fn dock_position(layout: &TaskbarLayout, win_w: i32, win_h: i32) -> (i32, i32) {
    match layout.edge {
        TaskbarEdge::Bottom => {
            // Place directly to the left of the notification area.
            let right_edge = layout
                .tray_notify
                .map(|r| r.x)
                .unwrap_or(layout.bar.x + layout.bar.w);
            let x = (right_edge - win_w).max(layout.bar.x);
            let y = if win_h >= layout.bar.h {
                layout.bar.y + layout.bar.h - win_h
            } else {
                layout.bar.y + (layout.bar.h - win_h) / 2
            };
            (x, y)
        }
        TaskbarEdge::Top => {
            let right_edge = layout
                .tray_notify
                .map(|r| r.x)
                .unwrap_or(layout.bar.x + layout.bar.w);
            let x = (right_edge - win_w).max(layout.bar.x);
            let y = if win_h >= layout.bar.h {
                layout.bar.y
            } else {
                layout.bar.y + (layout.bar.h - win_h) / 2
            };
            (x, y)
        }
        TaskbarEdge::Left => {
            // Place directly above the notification area.
            let bottom_edge = layout
                .tray_notify
                .map(|r| r.y)
                .unwrap_or(layout.bar.y + layout.bar.h);
            let y = (bottom_edge - win_h).max(layout.bar.y);
            let x = if win_w >= layout.bar.w {
                layout.bar.x
            } else {
                layout.bar.x + (layout.bar.w - win_w) / 2
            };
            (x, y)
        }
        TaskbarEdge::Right => {
            let bottom_edge = layout
                .tray_notify
                .map(|r| r.y)
                .unwrap_or(layout.bar.y + layout.bar.h);
            let y = (bottom_edge - win_h).max(layout.bar.y);
            let x = if win_w >= layout.bar.w {
                layout.bar.x + layout.bar.w - win_w
            } else {
                layout.bar.x + (layout.bar.w - win_w) / 2
            };
            (x, y)
        }
    }
}

/// Return the outer window size to use in taskbar mode. Horizontal taskbars
/// get a window as tall as the bar; vertical taskbars get a window as wide as
/// the bar. Content can still grow past that if the user chooses large text.
pub fn dock_size(layout: &TaskbarLayout, content_w: i32, content_h: i32) -> (i32, i32) {
    const MIN_W: i32 = 40;
    const MIN_H: i32 = 20;
    const MAX_HORIZONTAL_W: i32 = 1600;

    let content_w = content_w.max(MIN_W);
    let content_h = content_h.max(MIN_H);

    match layout.edge {
        TaskbarEdge::Bottom | TaskbarEdge::Top => {
            let (available_w, _) = layout.available_dock_size();
            let width = content_w.min(available_w).min(MAX_HORIZONTAL_W);
            (width, layout.bar.h.max(MIN_H))
        }
        TaskbarEdge::Left | TaskbarEdge::Right => {
            let (_, available_h) = layout.available_dock_size();
            let height = content_h.min(available_h);
            (layout.bar.w.max(MIN_W), height)
        }
    }
}

#[cfg(test)]
mod z_order_tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, GetForegroundWindow, PeekMessageW,
        TranslateMessage, MSG, PM_REMOVE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
        WS_POPUP, WS_VISIBLE,
    };

    struct TestWindow(HWND);

    impl TestWindow {
        fn new() -> Self {
            // Native off-screen windows exercise actual Windows Z ordering;
            // they neither activate nor interact with the real taskbar.
            Self(unsafe {
                CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    w!("STATIC"),
                    w!("SysPulse Z order regression"),
                    WS_POPUP | WS_VISIBLE,
                    -32000,
                    -32000,
                    1,
                    1,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap()
            })
        }

        fn raise(&self) {
            unsafe {
                SetWindowPos(
                    self.0,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                )
                .unwrap();
            }
        }
    }

    impl Drop for TestWindow {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }

    #[test]
    fn repairs_occlusion_without_activation_or_unnecessary_raising() {
        let foreground = unsafe { GetForegroundWindow() };
        let overlay = TestWindow::new();
        let taskbar = TestWindow::new();
        overlay.raise();
        assert!(!ensure_above_window(overlay.0, taskbar.0).unwrap());

        // Both already have WS_EX_TOPMOST. Raising the reference reproduces
        // the regression that setting the cached flag to true cannot repair.
        taskbar.raise();
        assert!(is_above_window(taskbar.0, overlay.0));
        let mut before = RECT::default();
        unsafe { GetWindowRect(overlay.0, &mut before).unwrap() };
        assert!(ensure_above_window(overlay.0, taskbar.0).unwrap());
        assert!(is_above_window(overlay.0, taskbar.0));
        let mut after = RECT::default();
        unsafe { GetWindowRect(overlay.0, &mut after).unwrap() };
        assert_eq!(before, after);
        assert_eq!(foreground, unsafe { GetForegroundWindow() });

        let menu = TestWindow::new();
        menu.raise();
        assert!(!ensure_above_window(overlay.0, taskbar.0).unwrap());
        assert!(is_above_window(menu.0, overlay.0));
        assert_eq!(foreground, unsafe { GetForegroundWindow() });
    }

    fn pump_events() {
        unsafe {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }

    #[test]
    fn shell_reordering_is_repaired_by_events_without_a_polling_timer() {
        let overlay = TestWindow::new();
        let taskbar = TestWindow::new();
        overlay.raise();
        watch_overlay_z_order(overlay.0).unwrap();
        struct StopHooks;
        impl Drop for StopHooks {
            fn drop(&mut self) {
                stop_overlay_z_order_watch();
            }
        }
        let _stop = StopHooks;
        Z_ORDER_HOOKS
            .with(|slot| slot.borrow_mut().as_mut().unwrap().reference_override = Some(taskbar.0));
        pump_events();
        let mut longest = std::time::Duration::ZERO;
        for _ in 0..30 {
            taskbar.raise();
            let start = std::time::Instant::now();
            while is_above_window(taskbar.0, overlay.0) && start.elapsed().as_millis() < 250 {
                pump_events();
                std::thread::yield_now();
            }
            assert!(
                is_above_window(overlay.0, taskbar.0),
                "no native reorder callback repaired the overlay"
            );
            longest = longest.max(start.elapsed());
        }
        eprintln!("30 shell reorder events: max restore latency {longest:?}");
        assert!(
            longest.as_millis() < 100,
            "event handling fell back to visible-delay behavior"
        );

        let menu = TestWindow::new();
        menu.raise();
        pump_events();
        assert!(
            is_above_window(menu.0, overlay.0),
            "show/reorder events hid the menu"
        );

        stop_overlay_z_order_watch();
        taskbar.raise();
        pump_events();
        assert!(
            is_above_window(taskbar.0, overlay.0),
            "hooks were not removed"
        );
    }
}
