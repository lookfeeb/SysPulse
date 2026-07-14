//! Native tracking tooltip for the taskbar-docked overlay.
//!
//! The overlay WebView is reparented into `Shell_TrayWnd` as a `WS_CHILD`.
//! Browser and WebView tooltip popups are unreliable in that configuration, so
//! detailed hover data is rendered by the Windows common-controls tooltip.

use std::sync::{Mutex, OnceLock};
use windows::core::{w, PWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::Controls::{
    InitCommonControlsEx, ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX, TOOLTIPS_CLASSW, TTF_CENTERTIP,
    TTF_TRACK, TTM_ADDTOOLW, TTM_SETMAXTIPWIDTH, TTM_TRACKACTIVATE, TTM_TRACKPOSITION,
    TTM_UPDATETIPTEXTW, TTS_ALWAYSTIP, TTS_NOPREFIX, TTTOOLINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, IsWindow, SendMessageW, HMENU, WINDOW_STYLE, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

#[derive(Default)]
struct TooltipState {
    hwnd: isize,
    owner: isize,
    text: Vec<u16>,
}

static TOOLTIP: OnceLock<Mutex<TooltipState>> = OnceLock::new();

fn state() -> &'static Mutex<TooltipState> {
    TOOLTIP.get_or_init(|| Mutex::new(TooltipState::default()))
}

pub fn show(owner: HWND, text: &str, screen_x: i32, screen_y: i32) -> windows::core::Result<()> {
    let mut state = state().lock().unwrap_or_else(|e| e.into_inner());
    let current = HWND(state.hwnd as *mut _);
    let needs_create = state.owner != owner.0 as isize
        || state.hwnd == 0
        || unsafe { !IsWindow(current).as_bool() };

    if needs_create {
        destroy_locked(&mut state);
        state.text = text.encode_utf16().chain(std::iter::once(0)).collect();
        state.hwnd = create(owner, state.text.as_mut_ptr())?.0 as isize;
        state.owner = owner.0 as isize;
    } else {
        let mut old_tool = tool_info(owner, state.text.as_mut_ptr());
        unsafe {
            SendMessageW(
                current,
                TTM_TRACKACTIVATE,
                WPARAM(0),
                LPARAM((&mut old_tool as *mut TTTOOLINFOW) as isize),
            );
        }
        state.text = text.encode_utf16().chain(std::iter::once(0)).collect();
    }

    let tooltip = HWND(state.hwnd as *mut _);
    let mut tool = tool_info(owner, state.text.as_mut_ptr());

    unsafe {
        SendMessageW(
            tooltip,
            TTM_UPDATETIPTEXTW,
            WPARAM(0),
            LPARAM((&mut tool as *mut TTTOOLINFOW) as isize),
        );
        SendMessageW(
            tooltip,
            TTM_TRACKPOSITION,
            WPARAM(0),
            LPARAM(pack_point(screen_x, screen_y)),
        );
        SendMessageW(
            tooltip,
            TTM_TRACKACTIVATE,
            WPARAM(1),
            LPARAM((&mut tool as *mut TTTOOLINFOW) as isize),
        );
    }
    Ok(())
}

pub fn hide() {
    let mut state = state().lock().unwrap_or_else(|e| e.into_inner());
    if state.hwnd == 0 || state.owner == 0 {
        return;
    }
    let tooltip = HWND(state.hwnd as *mut _);
    let owner = HWND(state.owner as *mut _);
    let mut tool = tool_info(owner, state.text.as_mut_ptr());
    unsafe {
        SendMessageW(
            tooltip,
            TTM_TRACKACTIVATE,
            WPARAM(0),
            LPARAM((&mut tool as *mut TTTOOLINFOW) as isize),
        );
    }
}

fn create(owner: HWND, text: *mut u16) -> windows::core::Result<HWND> {
    let controls = INITCOMMONCONTROLSEX {
        dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_WIN95_CLASSES,
    };
    if !unsafe { InitCommonControlsEx(&controls) }.as_bool() {
        return Err(windows::core::Error::from_win32());
    }

    let tooltip = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            TOOLTIPS_CLASSW,
            w!(""),
            WS_POPUP | WINDOW_STYLE(TTS_ALWAYSTIP | TTS_NOPREFIX),
            0,
            0,
            0,
            0,
            owner,
            HMENU::default(),
            HINSTANCE::default(),
            None,
        )?
    };

    let mut tool = tool_info(owner, text);
    let added = unsafe {
        SendMessageW(
            tooltip,
            TTM_ADDTOOLW,
            WPARAM(0),
            LPARAM((&mut tool as *mut TTTOOLINFOW) as isize),
        )
    };
    if added.0 == 0 {
        unsafe {
            let _ = DestroyWindow(tooltip);
        }
        return Err(windows::core::Error::from_win32());
    }

    unsafe {
        SendMessageW(tooltip, TTM_SETMAXTIPWIDTH, WPARAM(0), LPARAM(420));
    }
    Ok(tooltip)
}

fn tool_info(owner: HWND, text: *mut u16) -> TTTOOLINFOW {
    TTTOOLINFOW {
        cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_TRACK | TTF_CENTERTIP,
        hwnd: owner,
        uId: 1,
        rect: RECT::default(),
        hinst: HINSTANCE::default(),
        lpszText: PWSTR(text),
        lParam: LPARAM(0),
        lpReserved: std::ptr::null_mut(),
    }
}

fn pack_point(x: i32, y: i32) -> isize {
    let x = x as i16 as u16 as u32;
    let y = y as i16 as u16 as u32;
    ((y << 16) | x) as isize
}

fn destroy_locked(state: &mut TooltipState) {
    if state.hwnd != 0 {
        unsafe {
            let _ = DestroyWindow(HWND(state.hwnd as *mut _));
        }
    }
    *state = TooltipState::default();
}
