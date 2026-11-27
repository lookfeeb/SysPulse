//! Opt-in real Tauri/WebView2 lifecycle regression, using a private browser
//! profile and hidden windows. Does not start samplers or restart Explorer.
use super::*;
use crate::windows_api::taskbar::{OverlayRecovery, ShellIdentity};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{IsWindow, RegisterWindowMessageW, SendMessageW};

// A bounded test pump deliberately yields between iterations; run_return would
// not let the assertions drive destruction and recreation in the same runtime.
#[allow(deprecated)]
fn pump_until(app: &mut tauri::App, mut ready: impl FnMut(&tauri::App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !ready(app) {
        assert!(
            Instant::now() < deadline,
            "Tauri/WebView2 lifecycle timed out"
        );
        app.run_iteration(|_, _| {});
        std::thread::sleep(Duration::from_millis(10));
    }
}

// Deliberately contains no JavaScript double-click handler. A physical mouse
// must open the real config window through the native hook and production code.
#[test]
#[allow(deprecated)]
#[ignore = "interactive desktop fixture: double-click the overlay three times, closing config between rounds"]
fn physical_double_click_opens_hidden_minimized_and_recreated_overlay() {
    use crate::windows_api::taskbar::{dock_to_layout, PixelRect, TaskbarEdge, TaskbarLayout};
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetForegroundWindow};

    let profile =
        std::env::temp_dir().join(format!("syspulse-click-test-{}", uuid::Uuid::new_v4()));
    let mut context = tauri::generate_context!();
    for window in &mut context.config_mut().app.windows {
        window.url = tauri::WebviewUrl::External(
            format!("overlay-test://localhost/{}", window.label)
                .parse()
                .unwrap(),
        );
        window.visible = false;
        window.data_directory = Some(profile.clone());
        if window.label == CONFIG_LABEL {
            window.title = "SysPulse 主窗口验收 — 关闭后继续".into();
            window.width = 600.0;
            window.height = 360.0;
            window.min_width = None;
            window.min_height = None;
        }
    }
    let loaded = Arc::new(AtomicUsize::new(0));
    let loaded_callback = loaded.clone();
    let mut app = tauri::Builder::default().any_thread()
        .register_uri_scheme_protocol("overlay-test", |_, request| {
            let text = if request.uri().path() == "/config" {
                "主窗口已打开。关闭此窗口，继续下一轮双击验收。"
            } else { "双击这里打开主窗口（原生鼠标验收，无网页双击处理）" };
            tauri::http::Response::builder().header("Content-Type", "text/html; charset=utf-8")
                .body(format!("<html><meta charset='utf-8'><body style='margin:0;background:#e5f2ff;color:#163b61;font:18px sans-serif;display:flex;align-items:center;justify-content:center;height:100vh'>{text}</body></html>").into_bytes()).unwrap()
        })
        .on_page_load(move |webview, payload| {
            if webview.label() == OVERLAY_LABEL && payload.event() == tauri::webview::PageLoadEvent::Finished {
                loaded_callback.fetch_add(1, Ordering::Relaxed);
            }
        })
        .on_window_event(crate::app::on_window_event).build(context).unwrap();
    app.run_iteration(|_, _| {});
    pump_until(&mut app, |_| loaded.load(Ordering::Relaxed) >= 1);
    let surface = tauri::WebviewWindowBuilder::new(
        app.handle(),
        "test-taskbar",
        tauri::WebviewUrl::External("overlay-test://localhost/surface".parse().unwrap()),
    )
    .title("SysPulse 悬浮窗双击验收")
    .inner_size(620.0, 140.0)
    .position(100.0, 100.0)
    .build()
    .unwrap();
    let config = app.get_webview_window(CONFIG_LABEL).unwrap();
    let native_config = HWND(config.hwnd().unwrap().0 as *mut _);
    let native_surface = HWND(surface.hwnd().unwrap().0 as *mut _);
    let dock = |app: &tauri::App| {
        let overlay = app.get_webview_window(OVERLAY_LABEL).unwrap();
        watch_overlay_events(app.handle(), &overlay).unwrap();
        let mut rect = RECT::default();
        let mut origin = POINT::default();
        unsafe {
            GetClientRect(native_surface, &mut rect).unwrap();
            ClientToScreen(native_surface, &mut origin).ok().unwrap();
        }
        dock_to_layout(
            HWND(overlay.hwnd().unwrap().0 as *mut _),
            &TaskbarLayout {
                hwnd: native_surface,
                edge: TaskbarEdge::Bottom,
                bar: PixelRect {
                    x: origin.x,
                    y: origin.y,
                    w: rect.right,
                    h: rect.bottom,
                },
                tray_notify: None,
            },
            Some((600, 100)),
        )
        .unwrap();
    };
    let wait_for = |app: &mut tauri::App, ready: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(180);
        while !ready() {
            assert!(
                Instant::now() < deadline,
                "physical mouse verification timed out"
            );
            app.run_iteration(|_, _| {});
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    dock(&app);
    for round in 1..=3 {
        eprintln!(
            "READY round {round}: double-click the blue overlay, then close the config window"
        );
        wait_for(&mut app, &|| {
            config.is_visible().unwrap() && !config.is_minimized().unwrap()
        });
        assert_eq!(
            unsafe { GetForegroundWindow() },
            native_config,
            "main window must reach the foreground"
        );
        eprintln!("PASS round {round}: physical double-click showed and focused the main window");
        if round == 3 {
            break;
        }
        wait_for(&mut app, &|| !config.is_visible().unwrap());
        if round == 1 {
            config.show().unwrap();
            config.minimize().unwrap();
            pump_until(&mut app, |_| config.is_minimized().unwrap());
        } else {
            app.get_webview_window(OVERLAY_LABEL)
                .unwrap()
                .destroy()
                .unwrap();
            pump_until(&mut app, |app| {
                app.get_webview_window(OVERLAY_LABEL).is_none()
            });
            get_or_create_overlay_window(app.handle()).unwrap().unwrap();
            pump_until(&mut app, |_| loaded.load(Ordering::Relaxed) >= 2);
            dock(&app);
        }
    }
    for window in app.webview_windows().into_values() {
        window.destroy().unwrap();
    }
    pump_until(&mut app, |app| app.webview_windows().is_empty());
    drop(app);
    let _ = std::fs::remove_dir_all(profile);
}

#[test]
#[allow(deprecated)]
#[ignore = "requires Windows desktop and WebView2; run separately with --ignored --test-threads=1"]
fn shell_broadcast_recreates_a_live_tauri_host_and_settings_ipc_still_works() {
    let profile =
        std::env::temp_dir().join(format!("syspulse-overlay-test-{}", uuid::Uuid::new_v4()));
    let mut context = tauri::generate_context!();
    for window in &mut context.config_mut().app.windows {
        window.url = tauri::WebviewUrl::External("overlay-test://localhost".parse().unwrap());
        window.visible = false;
        window.center = false;
        window.x = Some(-30000.0);
        window.y = Some(-30000.0);
        window.data_directory = Some(profile.clone());
    }
    let loaded = Arc::new(AtomicUsize::new(0));
    let loaded_for_callback = loaded.clone();
    let mut app = tauri::Builder::default()
        .any_thread()
        .register_uri_scheme_protocol("overlay-test", |_, _| {
            tauri::http::Response::builder()
                .header("Content-Type", "text/html")
                .body(b"<html><body>SysPulse regression</body></html>".to_vec())
                .unwrap()
        })
        .on_page_load(move |webview, payload| {
            if webview.label() == OVERLAY_LABEL
                && payload.event() == tauri::webview::PageLoadEvent::Finished
            {
                loaded_for_callback.fetch_add(1, Ordering::Relaxed);
            }
        })
        .invoke_handler(tauri::generate_handler![show_config_window])
        .on_window_event(crate::app::on_window_event)
        .build(context)
        .unwrap();
    // run_iteration performs Tauri setup/window creation on the first pass.
    app.run_iteration(|_, _| {});
    pump_until(&mut app, |_| loaded.load(Ordering::Relaxed) >= 1);
    let config = app.get_webview_window(CONFIG_LABEL).unwrap();
    let native_config = HWND(config.hwnd().unwrap().0 as *mut _);
    crate::windows_api::taskbar::watch_taskbar_restarts(native_config).unwrap();
    let mut recovery = OverlayRecovery::new(ShellIdentity::current());
    assert!(
        ShellIdentity::current().is_some(),
        "requires a Windows shell"
    );

    for iteration in 0..2 {
        let overlay = app.get_webview_window(OVERLAY_LABEL).unwrap();
        let native_overlay = HWND(overlay.hwnd().unwrap().0 as *mut _);
        assert!(unsafe { IsWindow(native_overlay).as_bool() });
        unsafe {
            SendMessageW(
                native_config,
                RegisterWindowMessageW(w!("TaskbarCreated")),
                WPARAM(0),
                LPARAM(0),
            );
        }
        restore_taskbar_overlay(app.handle(), &mut recovery).unwrap();
        pump_until(&mut app, |app| {
            app.get_webview_window(OVERLAY_LABEL).is_none()
        });
        assert!(!unsafe { IsWindow(native_overlay).as_bool() });
        // Confirm the pending state clears only after runtime label removal.
        assert_eq!(
            recovery.observe(ShellIdentity::current(), None),
            crate::windows_api::taskbar::RecoveryAction::DockOrCreate
        );
        let replacement = get_or_create_overlay_window(app.handle()).unwrap().unwrap();
        pump_until(&mut app, |_| {
            loaded.load(Ordering::Relaxed) >= iteration + 2
        });
        // Exercise the same frontend -> Rust command used by double-click.
        replacement
            .eval("window.__TAURI_INTERNALS__.invoke('show_config_window')")
            .unwrap();
        pump_until(&mut app, |_| config.is_visible().unwrap());
        config.hide().unwrap();
    }
    for window in app.webview_windows().into_values() {
        window.destroy().unwrap();
    }
    pump_until(&mut app, |app| app.webview_windows().is_empty());
    drop(app);
    // WebView2 may release profile files asynchronously; removal is best effort.
    let _ = std::fs::remove_dir_all(profile);
}
