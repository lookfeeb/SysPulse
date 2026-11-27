//! WebView2 renders in foreign child HWNDs. Receive the physical click sequence
//! before Chromium dispatches DOM events, but only act on this overlay's HWNDs.
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;

const SUBCLASS_ID: usize = 0x5350_4443;
static OPEN_MESSAGE: OnceLock<u32> = OnceLock::new();
thread_local! {
    static INPUT: RefCell<Option<OverlayInput>> = const { RefCell::new(None) };
}

struct OverlayInput {
    hook: HHOOK,
    overlay: HWND,
    clicks: Clicks,
    open: Rc<dyn Fn()>,
}

impl Drop for OverlayInput {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWindowsHookEx(self.hook);
            let _ = RemoveWindowSubclass(self.overlay, Some(window_proc), SUBCLASS_ID);
        }
    }
}

/// Must be installed and removed on the UI message-loop thread.
pub fn watch(overlay: HWND, open: impl Fn() + 'static) -> Result<()> {
    let open: Rc<dyn Fn()> = Rc::new(open);
    let updated = INPUT.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(input) = slot.as_mut().filter(|input| input.overlay == overlay) {
            input.open = open.clone();
            return true;
        }
        false
    });
    if updated {
        return Ok(());
    }
    stop();
    let message = *OPEN_MESSAGE
        .get_or_init(|| unsafe { RegisterWindowMessageW(w!("SysPulse.OverlayDoubleClick")) });
    if message == 0 {
        return Err(windows::core::Error::from_win32());
    }
    unsafe {
        SetWindowSubclass(overlay, Some(window_proc), SUBCLASS_ID, 0).ok()?;
        let hook = match SetWindowsHookExW(
            WH_MOUSE_LL,
            Some(mouse_proc),
            GetModuleHandleW(PCWSTR::null())?,
            0,
        ) {
            Ok(hook) => hook,
            Err(error) => {
                let _ = RemoveWindowSubclass(overlay, Some(window_proc), SUBCLASS_ID);
                return Err(error);
            }
        };
        INPUT.with(|slot| {
            *slot.borrow_mut() = Some(OverlayInput {
                hook,
                overlay,
                clicks: Clicks::default(),
                open,
            })
        });
    }
    tracing::info!("overlay native double-click handler installed");
    Ok(())
}

pub fn stop() {
    // Drop outside the borrow: Win32 cleanup can reenter a window procedure.
    let input = INPUT.with(|slot| slot.borrow_mut().take());
    drop(input);
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let message = wparam.0 as u32;
    if code >= 0
        && matches!(
            message,
            WM_MOUSEMOVE
                | WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | WM_RBUTTONDOWN
                | WM_MBUTTONDOWN
                | WM_XBUTTONDOWN
        )
    {
        let target = INPUT.with(|slot| {
            slot.borrow()
                .as_ref()
                .map(|input| (input.overlay, input.clicks.pressed.is_some()))
        });
        if let Some((overlay, pressed)) = target {
            if message == WM_MOUSEMOVE && !pressed {
                return CallNextHookEx(None, code, wparam, lparam);
            }
            let event = &*(lparam.0 as *const MSLLHOOKSTRUCT);
            // Mouse moves only matter during a press (drag rejection).
            let needs_hit = message == WM_LBUTTONDOWN || message == WM_LBUTTONUP;
            let over = needs_hit
                && IsWindowVisible(overlay).as_bool()
                && super::is_overlay_hit(overlay, WindowFromPoint(event.pt));
            let recognized = INPUT.with(|slot| {
                slot.borrow_mut().as_mut().is_some_and(|input| {
                    input.clicks.feed(
                        message,
                        event.pt,
                        event.time,
                        over,
                        GetDoubleClickTime(),
                        (
                            GetSystemMetrics(SM_CXDOUBLECLK).max(2) / 2,
                            GetSystemMetrics(SM_CYDOUBLECLK).max(2) / 2,
                        ),
                    )
                })
            });
            if recognized {
                // Never show/focus a window or run IPC inside a low-level hook.
                // Post to our UI procedure, then immediately pass input onward.
                let _ = PostMessageW(overlay, *OPEN_MESSAGE.get().unwrap(), WPARAM(0), LPARAM(0));
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_MOUSEACTIVATE {
        // A child otherwise forwards activation to Explorer. Preserve shell
        // focus while allowing WebView2 to receive ordinary/context-menu clicks.
        return LRESULT(MA_NOACTIVATE as isize);
    }
    if OPEN_MESSAGE.get().copied() == Some(message) {
        let open = INPUT.with(|slot| {
            slot.borrow()
                .as_ref()
                .filter(|input| input.overlay == window)
                .map(|input| input.open.clone())
        });
        if let Some(open) = open {
            open();
        }
        return LRESULT(0);
    }
    if message == WM_NCDESTROY {
        let _ = RemoveWindowSubclass(window, Some(window_proc), id);
    }
    DefSubclassProc(window, message, wparam, lparam)
}

#[derive(Clone, Copy)]
struct Press {
    point: POINT,
    time: u32,
}

#[derive(Default)]
struct Clicks {
    first: Option<Press>,
    pressed: Option<Press>,
    second: bool,
}

impl Clicks {
    fn feed(
        &mut self,
        message: u32,
        point: POINT,
        time: u32,
        over: bool,
        interval: u32,
        tolerance: (i32, i32),
    ) -> bool {
        let close = |a: POINT, b: POINT| {
            (a.x as i64 - b.x as i64).abs() <= tolerance.0 as i64
                && (a.y as i64 - b.y as i64).abs() <= tolerance.1 as i64
        };
        match message {
            WM_LBUTTONDOWN if over => {
                self.second = self.first.is_some_and(|first| {
                    time.wrapping_sub(first.time) <= interval && close(first.point, point)
                });
                self.first = None;
                self.pressed = Some(Press { point, time });
            }
            WM_LBUTTONUP if over => {
                if let Some(pressed) = self.pressed.take().filter(|p| close(p.point, point)) {
                    if self.second {
                        *self = Self::default();
                        return true;
                    }
                    self.first = Some(pressed);
                } else {
                    *self = Self::default();
                }
            }
            WM_MOUSEMOVE => {
                if self.pressed.is_some_and(|p| !close(p.point, point)) {
                    *self = Self::default();
                }
            }
            _ => *self = Self::default(),
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click(clicks: &mut Clicks, time: u32, x: i32, over: bool) -> bool {
        let point = POINT { x, y: -100 };
        assert!(!clicks.feed(WM_LBUTTONDOWN, point, time, over, 500, (4, 4)));
        clicks.feed(
            WM_LBUTTONUP,
            point,
            time.wrapping_add(20),
            over,
            500,
            (4, 4),
        )
    }

    #[test]
    fn two_completed_clicks_open_once_including_across_tick_wrap() {
        for start in [100, u32::MAX - 80] {
            let mut clicks = Clicks::default();
            assert!(!click(&mut clicks, start, -200, true));
            assert!(click(&mut clicks, start.wrapping_add(150), -198, true));
            assert!(!click(&mut clicks, start.wrapping_add(220), -198, true));
        }
    }

    #[test]
    fn slow_distant_outside_and_other_button_clicks_do_not_open() {
        for (delay, x, over) in [(501, 10, true), (150, 20, true), (150, 10, false)] {
            let mut clicks = Clicks::default();
            assert!(!click(&mut clicks, 100, 10, true));
            assert!(!click(&mut clicks, 100 + delay, x, over));
        }
        let mut clicks = Clicks::default();
        assert!(!click(&mut clicks, 100, 10, true));
        clicks.feed(
            WM_RBUTTONDOWN,
            POINT { x: 10, y: -100 },
            150,
            true,
            500,
            (4, 4),
        );
        assert!(!click(&mut clicks, 200, 10, true));
    }

    #[test]
    fn dragging_out_and_back_cancels_the_sequence() {
        let mut clicks = Clicks::default();
        let origin = POINT { x: 10, y: -100 };
        clicks.feed(WM_LBUTTONDOWN, origin, 100, true, 500, (4, 4));
        clicks.feed(
            WM_MOUSEMOVE,
            POINT { x: 30, ..origin },
            110,
            false,
            500,
            (4, 4),
        );
        assert!(!clicks.feed(WM_LBUTTONUP, origin, 120, true, 500, (4, 4)));
        assert!(!click(&mut clicks, 200, 10, true));
    }
}
