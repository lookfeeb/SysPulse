//! Hide tiny helper windows created by tao / plugins. They are message targets,
//! not part of the user-facing UI, but Windows may still mark them visible.

use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
    ShowWindow, SW_HIDE,
};

struct HideContext {
    process_id: u32,
    hidden: u32,
}

pub fn hide_auxiliary_windows() {
    unsafe {
        let mut ctx = HideContext {
            process_id: GetCurrentProcessId(),
            hidden: 0,
        };
        let _ = EnumWindows(
            Some(enum_window),
            LPARAM((&mut ctx as *mut HideContext) as isize),
        );
        if ctx.hidden > 0 {
            tracing::debug!(hidden = ctx.hidden, "hid auxiliary windows");
        }
    }
}

unsafe extern "system" fn enum_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut HideContext);

    let mut window_process_id = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut window_process_id));
    if window_process_id != ctx.process_id || !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }

    let class = class_name(hwnd);
    if class == "Tauri Window" {
        return BOOL(1);
    }

    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_err() {
        return BOOL(1);
    }

    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 32 && height <= 32 {
        let _ = ShowWindow(hwnd, SW_HIDE);
        ctx.hidden += 1;
    }

    BOOL(1)
}

unsafe fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let len = GetClassNameW(hwnd, &mut buf);
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}
