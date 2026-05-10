//! Helpers for locating the Windows shell taskbar (`Shell_TrayWnd`) so we can
//! position the overlay window in its notification area.
//!
//! True COM-based deskbands are deprecated on Windows 11. We instead use a
//! topmost popup window sized to the taskbar and placed precisely where the
//! deskband would have lived.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, GetWindowRect};

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
    pub hwnd: HWND,
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
            hwnd: taskbar,
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
        let clock = FindWindowExW(
            hwnd,
            HWND::default(),
            w!("TrayClockWClass"),
            PCWSTR::null(),
        )
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
