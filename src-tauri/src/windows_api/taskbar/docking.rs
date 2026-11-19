use super::{dock_position, dock_size, find_taskbar_layout, TaskbarLayout};
use windows::core::Result;
use windows::Win32::Foundation::{
    GetLastError, SetLastError, ERROR_INVALID_WINDOW_HANDLE, HWND, POINT, RECT, WIN32_ERROR,
};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::WindowsAndMessaging::{
    GetParent, GetWindow, GetWindowLongPtrW, GetWindowRect, IsChild, IsWindow, SetParent,
    SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE, GW_HWNDPREV, HWND_NOTOPMOST, HWND_TOP,
    HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    WS_CHILD, WS_CLIPSIBLINGS, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP, WS_VISIBLE,
};

/// All parenting, sizing and positioning runs on the window's UI thread.
/// Do not use Tao's cached popup flags or screen-coordinate setters afterwards.
pub fn dock_overlay(overlay: HWND, content_size: Option<(i32, i32)>) -> Result<()> {
    let Some(layout) = find_taskbar_layout() else {
        // Explorer may be restarting. The watchdog retries with its new HWND.
        return Ok(());
    };
    dock_to_layout(overlay, &layout, content_size).map(|_| ())
}

fn dock_to_layout(
    overlay: HWND,
    layout: &TaskbarLayout,
    content_size: Option<(i32, i32)>,
) -> Result<bool> {
    let mut before = RECT::default();
    unsafe { GetWindowRect(overlay, &mut before)? };
    let (content_w, content_h) =
        content_size.unwrap_or((before.right - before.left, before.bottom - before.top));
    let (width, height) = dock_size(layout, content_w, content_h);
    let (x, y) = dock_position(layout, width, height);

    let attached = match attach_to_taskbar(overlay, layout.hwnd) {
        Ok(changed) => changed,
        Err(error) => {
            // Windows can reject cross-band parenting while Start is already
            // open. Keep a usable popup and retry when the shell returns to its
            // desktop band; never leave a half-converted child window behind.
            tracing::debug!(?error, "taskbar attachment deferred; retaining popup");
            unsafe {
                SetWindowPos(
                    overlay,
                    HWND_TOPMOST,
                    x,
                    y,
                    width,
                    height,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )?;
            }
            return Ok(true);
        }
    };
    let mut origin = POINT { x, y };
    unsafe { ScreenToClient(layout.hwnd, &mut origin).ok()? };
    let visible = unsafe { GetWindowLongPtrW(overlay, GWL_STYLE) as u32 & WS_VISIBLE.0 != 0 };
    if !attached
        && visible
        && before
            == (RECT {
                left: x,
                top: y,
                right: x + width,
                bottom: y + height,
            })
    {
        return raise_child_if_needed(overlay);
    }
    let mut flags = SWP_NOACTIVATE;
    if attached {
        flags |= SWP_FRAMECHANGED;
    }
    if !visible {
        flags |= SWP_SHOWWINDOW;
    }
    unsafe {
        SetWindowPos(overlay, HWND_TOP, origin.x, origin.y, width, height, flags)?;
    }
    Ok(true)
}

pub(super) fn is_taskbar_child(overlay: HWND, taskbar: HWND) -> bool {
    unsafe {
        GetWindowLongPtrW(overlay, GWL_STYLE) as u32 & WS_CHILD.0 != 0
            && GetParent(overlay).ok() == Some(taskbar)
    }
}

fn attach_to_taskbar(overlay: HWND, taskbar: HWND) -> Result<bool> {
    if is_taskbar_child(overlay, taskbar) {
        return Ok(false);
    }
    unsafe {
        if !IsWindow(taskbar).as_bool() {
            return Err(windows::core::Error::from(ERROR_INVALID_WINDOW_HANDLE));
        }
        let style = GetWindowLongPtrW(overlay, GWL_STYLE);
        let ex_style = GetWindowLongPtrW(overlay, GWL_EXSTYLE);
        let child_style = (style as u32 | WS_CHILD.0 | WS_CLIPSIBLINGS.0) & !WS_POPUP.0;
        let child_ex_style = (ex_style as u32 | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0)
            & !(WS_EX_APPWINDOW.0 | WS_EX_TOPMOST.0);
        // WS_EX_TOPMOST is managed by SetWindowPos; writing the style bit alone
        // leaves Windows' internal topmost state behind even after SetParent.
        SetWindowPos(
            overlay,
            HWND_NOTOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
        )?;
        let result = set_window_long(overlay, GWL_STYLE, child_style as isize)
            .and_then(|_| set_window_long(overlay, GWL_EXSTYLE, child_ex_style as isize))
            .and_then(|_| SetParent(overlay, taskbar).map(|_| ()));
        if let Err(error) = result {
            let _ = set_window_long(overlay, GWL_STYLE, style);
            let _ = set_window_long(overlay, GWL_EXSTYLE, ex_style);
            if ex_style as u32 & WS_EX_TOPMOST.0 != 0 {
                let _ = SetWindowPos(
                    overlay,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                );
            }
            return Err(error);
        }
    }
    Ok(true)
}

unsafe fn set_window_long(
    hwnd: HWND,
    index: windows::Win32::UI::WindowsAndMessaging::WINDOW_LONG_PTR_INDEX,
    value: isize,
) -> Result<()> {
    SetLastError(WIN32_ERROR(0));
    let previous = SetWindowLongPtrW(hwnd, index, value);
    if previous == 0 && GetLastError().0 != 0 {
        return Err(windows::core::Error::from_win32());
    }
    Ok(())
}

pub(super) fn raise_child_if_needed(overlay: HWND) -> Result<bool> {
    // EnumWindows excludes children (and the taskbar in Start's shell band).
    // Check sibling order instead, and avoid repainting an unchanged WebView.
    if unsafe { GetWindow(overlay, GW_HWNDPREV) }.is_err() {
        return Ok(false);
    }
    unsafe {
        SetWindowPos(
            overlay,
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
        )?;
    }
    Ok(true)
}

/// WebView2 has nested child HWNDs. GA_ROOT now resolves to Explorer, so it
/// cannot be used to decide whether the pointer is over the overlay itself.
pub fn is_overlay_hit(overlay: HWND, hit: HWND) -> bool {
    overlay == hit || unsafe { IsChild(overlay, hit).as_bool() }
}

#[cfg(test)]
mod tests {
    use super::super::{PixelRect, TaskbarEdge};
    use super::*;
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, GetForegroundWindow, IsWindowVisible,
    };

    struct Window(HWND);

    impl Window {
        fn new(parent: Option<HWND>) -> Self {
            Self(unsafe {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    w!("STATIC"),
                    w!("SysPulse taskbar docking regression"),
                    WS_VISIBLE | if parent.is_some() { WS_CHILD } else { WS_POPUP },
                    -32000,
                    -32000,
                    1,
                    1,
                    parent.unwrap_or_default(),
                    None,
                    None,
                    None,
                )
                .unwrap()
            })
        }

        fn layout(&self) -> TaskbarLayout {
            unsafe {
                SetWindowPos(self.0, HWND_TOP, -30000, -30000, 800, 48, SWP_NOACTIVATE).unwrap();
            }
            TaskbarLayout {
                hwnd: self.0,
                edge: TaskbarEdge::Bottom,
                bar: PixelRect {
                    x: -30000,
                    y: -30000,
                    w: 800,
                    h: 48,
                },
                tray_notify: Some(PixelRect {
                    x: -29300,
                    y: -30000,
                    w: 100,
                    h: 48,
                }),
            }
        }
    }

    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }

    #[test]
    fn child_docking_keeps_screen_coordinates_and_does_not_repaint_or_activate() {
        let foreground = unsafe { GetForegroundWindow() };
        let taskbar = Window::new(None);
        let overlay = Window::new(None);
        let layout = taskbar.layout();
        unsafe {
            SetWindowPos(
                overlay.0,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            )
            .unwrap();
        }
        assert!(dock_to_layout(overlay.0, &layout, Some((240, 32))).unwrap());
        assert!(is_taskbar_child(overlay.0, taskbar.0));
        let mut rect = RECT::default();
        unsafe {
            GetWindowRect(overlay.0, &mut rect).unwrap();
        }
        assert_eq!(
            rect,
            RECT {
                left: -29540,
                top: -30000,
                right: -29300,
                bottom: -29952
            }
        );
        let style = unsafe { GetWindowLongPtrW(overlay.0, GWL_STYLE) as u32 };
        let ex_style = unsafe { GetWindowLongPtrW(overlay.0, GWL_EXSTYLE) as u32 };
        assert_eq!(style & WS_POPUP.0, 0);
        assert_eq!(ex_style & WS_EX_TOPMOST.0, 0);
        assert!(unsafe { IsWindowVisible(overlay.0).as_bool() });
        for _ in 0..30 {
            assert!(!dock_to_layout(overlay.0, &layout, None).unwrap());
        }
        assert_eq!(foreground, unsafe { GetForegroundWindow() });
    }

    #[test]
    fn repairs_child_order_and_accepts_only_overlay_descendants_for_hover() {
        let taskbar = Window::new(None);
        let overlay = Window::new(None);
        let layout = taskbar.layout();
        dock_to_layout(overlay.0, &layout, Some((240, 32))).unwrap();
        let webview = Window::new(Some(overlay.0));
        let renderer = Window::new(Some(webview.0));
        let sibling = Window::new(Some(taskbar.0));
        unsafe {
            SetWindowPos(
                sibling.0,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            )
            .unwrap();
        }
        assert!(raise_child_if_needed(overlay.0).unwrap());
        assert!(!raise_child_if_needed(overlay.0).unwrap());
        assert!(is_overlay_hit(overlay.0, overlay.0));
        assert!(is_overlay_hit(overlay.0, renderer.0));
        assert!(!is_overlay_hit(overlay.0, sibling.0));
        assert!(!is_overlay_hit(overlay.0, taskbar.0));
    }

    #[test]
    fn failed_attachment_preserves_popup_and_replacement_windows_can_dock() {
        let overlay = Window::new(None);
        let style = unsafe { GetWindowLongPtrW(overlay.0, GWL_STYLE) };
        let ex_style = unsafe { GetWindowLongPtrW(overlay.0, GWL_EXSTYLE) };
        assert!(attach_to_taskbar(overlay.0, HWND(1usize as *mut _)).is_err());
        assert!(attach_to_taskbar(overlay.0, overlay.0).is_err());
        assert_eq!(style, unsafe { GetWindowLongPtrW(overlay.0, GWL_STYLE) });
        assert_eq!(ex_style, unsafe {
            GetWindowLongPtrW(overlay.0, GWL_EXSTYLE)
        });
        let taskbar = Window::new(None);
        dock_to_layout(overlay.0, &taskbar.layout(), Some((240, 32))).unwrap();
        drop(taskbar);
        assert!(!unsafe { IsWindow(overlay.0).as_bool() });
        let replacement_taskbar = Window::new(None);
        let replacement_overlay = Window::new(None);
        dock_to_layout(
            replacement_overlay.0,
            &replacement_taskbar.layout(),
            Some((240, 32)),
        )
        .unwrap();
        assert!(is_taskbar_child(
            replacement_overlay.0,
            replacement_taskbar.0
        ));
    }
}
