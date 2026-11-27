//! Explorer can invalidate WebView2 without destroying its Tauri host HWND.
//! Track the shell lifetime, not just the presence of a Tauri window label.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    ChangeWindowMessageFilterEx, FindWindowW, GetWindowThreadProcessId, RegisterWindowMessageW,
    MSGFLT_ALLOW, WM_NCDESTROY,
};

static SHELL_GENERATION: AtomicU64 = AtomicU64::new(0);
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();
const SHELL_SUBCLASS: usize = 0x5350_5348;

/// Install on a top-level window: docked children do not receive broadcasts.
pub fn watch_taskbar_restarts(window: HWND) -> Result<()> {
    let message =
        *TASKBAR_CREATED.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) });
    if message == 0 {
        return Err(windows::core::Error::from_win32());
    }
    unsafe {
        // Release builds run elevated; Explorer's broadcast crosses UIPI.
        ChangeWindowMessageFilterEx(window, message, MSGFLT_ALLOW, None)?;
        SetWindowSubclass(window, Some(shell_proc), SHELL_SUBCLASS, 0).ok()
    }
}

unsafe extern "system" fn shell_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    _data: usize,
) -> LRESULT {
    if TASKBAR_CREATED.get().copied() == Some(message) {
        SHELL_GENERATION.fetch_add(1, Ordering::Relaxed);
    }
    if message == WM_NCDESTROY {
        let _ = RemoveWindowSubclass(window, Some(shell_proc), id);
    }
    DefSubclassProc(window, message, wparam, lparam)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShellIdentity {
    window: isize,
    process: u32,
    generation: u64,
}

impl ShellIdentity {
    pub fn current() -> Option<Self> {
        unsafe {
            let window = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()).ok()?;
            let mut process = 0;
            if GetWindowThreadProcessId(window, Some(&mut process)) == 0 {
                return None;
            }
            Some(Self {
                window: window.0 as isize,
                process,
                generation: SHELL_GENERATION.load(Ordering::Relaxed),
            })
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RecoveryAction {
    Wait,
    Destroy,
    DockOrCreate,
}

pub struct OverlayRecovery {
    shell: Option<ShellIdentity>,
    invalidated: bool,
    destroying: bool,
}

impl OverlayRecovery {
    pub fn new(shell: Option<ShellIdentity>) -> Self {
        Self {
            shell,
            invalidated: false,
            destroying: false,
        }
    }

    /// None means Tauri has removed the label; Some(false) is a stale HWND.
    pub fn observe(
        &mut self,
        shell: Option<ShellIdentity>,
        window_valid: Option<bool>,
    ) -> RecoveryAction {
        if self.shell.is_some() && self.shell != shell {
            self.invalidated = true;
        }
        self.shell = shell;
        if window_valid.is_none() {
            self.invalidated = false;
            self.destroying = false;
        } else {
            self.invalidated |= window_valid == Some(false);
            if self.destroying {
                return RecoveryAction::Wait;
            }
            if self.invalidated {
                return RecoveryAction::Destroy;
            }
        }
        if shell.is_some() {
            RecoveryAction::DockOrCreate
        } else {
            RecoveryAction::Wait
        }
    }

    pub fn destroy_requested(&mut self) {
        self.destroying = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, SendMessageW, WS_EX_TOOLWINDOW, WS_POPUP,
    };

    #[test]
    fn top_level_listener_records_taskbar_created_broadcasts() {
        unsafe {
            let window = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                w!("STATIC"),
                w!("SysPulse shell listener regression"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            watch_taskbar_restarts(window).unwrap();
            let before = SHELL_GENERATION.load(Ordering::Relaxed);
            SendMessageW(
                window,
                *TASKBAR_CREATED.get().unwrap(),
                WPARAM(0),
                LPARAM(0),
            );
            assert_eq!(SHELL_GENERATION.load(Ordering::Relaxed), before + 1);
            DestroyWindow(window).unwrap();
        }
    }

    fn shell(window: isize, process: u32, generation: u64) -> Option<ShellIdentity> {
        Some(ShellIdentity {
            window,
            process,
            generation,
        })
    }

    #[test]
    fn shell_restart_retires_even_a_live_host_and_waits_for_tauri_removal() {
        let old = shell(10, 100, 0);
        let new = shell(20, 200, 1);
        let mut recovery = OverlayRecovery::new(old);
        assert_eq!(
            recovery.observe(old, Some(true)),
            RecoveryAction::DockOrCreate
        );
        assert_eq!(recovery.observe(None, Some(true)), RecoveryAction::Destroy);
        // A failed destroy must be retried, even after the shell returns.
        assert_eq!(recovery.observe(new, Some(true)), RecoveryAction::Destroy);
        recovery.destroy_requested();
        assert_eq!(recovery.observe(new, Some(true)), RecoveryAction::Wait);
        assert_eq!(recovery.observe(new, None), RecoveryAction::DockOrCreate);
        assert_eq!(
            recovery.observe(new, Some(true)),
            RecoveryAction::DockOrCreate
        );
    }

    #[test]
    fn detects_restarts_between_ticks_and_reused_handles() {
        let old = shell(10, 100, 0);
        for new in [shell(20, 100, 0), shell(10, 200, 0), shell(10, 100, 1)] {
            let mut recovery = OverlayRecovery::new(old);
            assert_eq!(recovery.observe(new, Some(true)), RecoveryAction::Destroy);
        }
    }

    #[test]
    fn missing_shell_does_not_create_windows_and_stale_handles_are_retired() {
        let mut recovery = OverlayRecovery::new(None);
        assert_eq!(recovery.observe(None, None), RecoveryAction::Wait);
        let current = shell(10, 100, 0);
        assert_eq!(
            recovery.observe(current, None),
            RecoveryAction::DockOrCreate
        );
        assert_eq!(
            recovery.observe(current, Some(false)),
            RecoveryAction::Destroy
        );
        recovery.destroy_requested();
        assert_eq!(recovery.observe(None, None), RecoveryAction::Wait);
        assert_eq!(
            recovery.observe(current, None),
            RecoveryAction::DockOrCreate
        );
    }
}
