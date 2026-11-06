//! Lightweight, fully painted hover card. All window operations run on the UI thread.
mod drawing;

use drawing::{Style, Typography};
use std::cell::{Cell, RefCell};
use std::sync::OnceLock;
use std::time::Instant;
use windows::core::{w, Result};
use windows::Win32::Foundation::{
    BOOL, COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

thread_local! {
    static WINDOW: Cell<(isize, isize)> = const { Cell::new((0, 0)) };
    static FRAME: RefCell<Option<PopupFrame>> = const { RefCell::new(None) };
}

const INK: COLORREF = rgb(30, 41, 59);
const MUTED: COLORREF = rgb(71, 85, 105);
const BORDER: COLORREF = rgb(219, 228, 239);
const SURFACE: COLORREF = rgb(250, 252, 255);
const LIGHT_TIMER: usize = 1;

const fn rgb(r: u32, g: u32, b: u32) -> COLORREF {
    COLORREF(r | (g << 8) | (b << 16))
}

fn px(value: i32, dpi: u32) -> i32 {
    ((value as f64 * dpi as f64 / 96.0).round() as i32).max(1)
}

/// A top-down 32-bit bitmap. UpdateLayeredWindow copies the complete premultiplied
/// frame, so the compositor can blend corner coverage instead of clipping pixels.
struct LayerSurface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u8,
    width: i32,
    height: i32,
}

impl LayerSurface {
    fn new(width: i32, height: i32) -> Result<Self> {
        unsafe {
            let dc = CreateCompatibleDC(HDC::default());
            if dc.is_invalid() {
                return Err(windows::core::Error::from_win32());
            }
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = match CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(bitmap) => bitmap,
                Err(error) => {
                    let _ = DeleteDC(dc);
                    return Err(error);
                }
            };
            let previous = SelectObject(dc, bitmap);
            Ok(Self {
                dc,
                bitmap,
                previous,
                bits: bits.cast(),
                width,
                height,
            })
        }
    }

    #[cfg(test)]
    fn pixels(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.bits, (self.width * self.height * 4) as usize) }
    }

    fn smooth_edges(&mut self, radius: i32, border: i32) {
        let width = self.width as f64;
        let height = self.height as f64;
        let radius = (radius as f64).min(width / 2.0).min(height / 2.0);
        // Finish GDI's writes before accessing the DIB directly.
        unsafe {
            let _ = GdiFlush();
        }
        let pixels = unsafe {
            std::slice::from_raw_parts_mut(self.bits, (self.width * self.height * 4) as usize)
        };
        let border_bgr = [
            (BORDER.0 >> 16) & 255,
            (BORDER.0 >> 8) & 255,
            BORDER.0 & 255,
        ];
        for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
            let x = (index % self.width as usize) as f64 + 0.5;
            let y = (index / self.width as usize) as f64 + 0.5;
            let qx = (x - width / 2.0).abs() - (width / 2.0 - radius);
            let qy = (y - height / 2.0).abs() - (height / 2.0 - radius);
            let distance = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius;
            let coverage = (0.5 - distance).clamp(0.0, 1.0);
            let inner = (0.5 - distance - border as f64).clamp(0.0, 1.0);
            for channel in 0..3 {
                pixel[channel] = (pixel[channel] as f64 * inner
                    + border_bgr[channel] as f64 * (coverage - inner))
                    .round() as u8;
            }
            pixel[3] = (255.0 * coverage).round() as u8;
        }
    }

    fn paint_light(&mut self, dpi: u32, seconds: f64) {
        let scale = dpi as f64 / 96.0;
        let cx = px(20, dpi) as f64;
        let cy = px(24, dpi) as f64;
        let extent = px(10, dpi);
        let phase = (seconds / 2.4).fract();
        let breath = 0.5 - 0.5 * (phase * std::f64::consts::TAU).cos();
        let pixels = unsafe {
            std::slice::from_raw_parts_mut(self.bits, (self.width * self.height * 4) as usize)
        };
        // Only this small, opaque header patch changes. Text and rounded edges
        // stay in the cached frame and are never resampled by the animation.
        for y in ((cy as i32 - extent).max(0))..(cy as i32 + extent).min(self.height) {
            for x in ((cx as i32 - extent).max(0))..(cx as i32 + extent).min(self.width) {
                let dx = x as f64 + 0.5 - cx;
                let dy = y as f64 + 0.5 - cy;
                let distance = dx.hypot(dy);
                let core = (3.3 * scale + 0.5 - distance).clamp(0.0, 1.0);
                let glow = (-2.0 * (distance / ((6.0 + breath * 2.0) * scale)).powi(2)).exp()
                    * (0.18 + 0.20 * breath);
                let ring = (1.0 - (distance - (4.5 + phase * 4.0) * scale).abs() / scale).max(0.0)
                    * (1.0 - phase).powi(2)
                    * 0.24;
                let coverage = 1.0 - (1.0 - core) * (1.0 - glow) * (1.0 - ring);
                let highlight = ((dx + scale).hypot(dy + scale) / (1.1 * scale)).min(1.0);
                let highlight = (1.0 - highlight) * 0.42 * core;
                let pixel = &mut pixels[((y * self.width + x) * 4) as usize..][..4];
                for (channel, (background, blue)) in [(255.0, 224.0), (252.0, 144.0), (250.0, 24.0)]
                    .into_iter()
                    .enumerate()
                {
                    let value = background * (1.0 - coverage) + blue * coverage;
                    pixel[channel] = (value * (1.0 - highlight) + 255.0 * highlight).round() as u8;
                }
                pixel[3] = 255;
            }
        }
    }

    fn upload(&self, hwnd: HWND, left: i32, top: i32) -> Result<()> {
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            UpdateLayeredWindow(
                hwnd,
                HDC::default(),
                Some(&POINT { x: left, y: top }),
                Some(&SIZE {
                    cx: self.width,
                    cy: self.height,
                }),
                self.dc,
                Some(&POINT::default()),
                COLORREF::default(),
                Some(&blend),
                ULW_ALPHA,
            )
        }
    }
}

impl Drop for LayerSurface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap);
            let _ = DeleteDC(self.dc);
        }
    }
}

struct PopupFrame {
    surface: LayerSurface,
    dpi: u32,
    left: i32,
    top: i32,
    started: Instant,
    animated: bool,
}

fn animations_enabled() -> bool {
    let mut enabled = BOOL(1);
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&mut enabled as *mut BOOL).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    enabled.as_bool()
}

fn clear_frame(hwnd: HWND) {
    unsafe {
        let _ = KillTimer(hwnd, LIGHT_TIMER);
    }
    FRAME.with(|frame| {
        frame.borrow_mut().take();
    });
}

fn animate_light(hwnd: HWND) -> Result<()> {
    FRAME.with(|frame| {
        let mut frame = frame.borrow_mut();
        if let Some(frame) = frame.as_mut().filter(|frame| frame.animated) {
            frame
                .surface
                .paint_light(frame.dpi, frame.started.elapsed().as_secs_f64());
            if let Err(error) = frame.surface.upload(hwnd, frame.left, frame.top) {
                frame.animated = false;
                unsafe {
                    let _ = KillTimer(hwnd, LIGHT_TIMER);
                }
                return Err(error);
            }
        }
        Ok(())
    })
}

struct Row {
    label: String,
    value: String,
    height: i32,
}

struct Card {
    title: String,
    rows: Vec<Row>,
    typography: Typography,
    dpi: u32,
    width: i32,
    height: i32,
    label_width: i32,
}

impl Card {
    fn new(text: &str, dpi: u32, max_width: i32) -> Result<Self> {
        let mut lines = text.lines();
        let title = lines.next().unwrap_or("SysPulse").to_string();
        let mut rows: Vec<Row> = lines
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let (label, value) = line.split_once('：').unwrap_or(("", line));
                Row {
                    label: label.into(),
                    value: value.into(),
                    height: 0,
                }
            })
            .collect();
        let typography = Typography::new(dpi)?;
        let measure =
            |text: &str, style| typography.measure(text, style, 100_000).map(|size| size.0);
        let mut label_width = 0;
        let mut natural_width = 0;
        for row in &rows {
            label_width = label_width.max(measure(&row.label, Style::Body)?);
        }
        label_width = label_width.min(px(96, dpi));
        let padding = px(16, dpi);
        let gap = px(18, dpi);
        for row in &rows {
            natural_width = natural_width.max(
                measure(&row.value, Style::Body)?
                    + if row.label.is_empty() {
                        0
                    } else {
                        label_width + gap
                    },
            );
        }
        let width = (natural_width + padding * 2)
            .max(px(264, dpi))
            .max(measure(&title, Style::Title)? + px(128, dpi))
            .min(max_width.max(1));
        for row in &mut rows {
            let value_width = width
                - padding * 2
                - if row.label.is_empty() {
                    0
                } else {
                    label_width + gap
                };
            row.height = typography
                .measure(&row.value, Style::Body, value_width)?
                .1
                .max(typography.measure(&row.label, Style::Body, label_width)?.1)
                .max(px(19, dpi))
                + px(7, dpi);
        }
        let height = px(52, dpi) + rows.iter().map(|row| row.height).sum::<i32>();
        Ok(Self {
            title,
            rows,
            typography,
            dpi,
            width,
            height,
            label_width,
        })
    }

    fn render(&self) -> Result<LayerSurface> {
        let mut surface = LayerSurface::new(self.width, self.height)?;
        self.paint(surface.dc)?;
        surface.smooth_edges(px(10, self.dpi), px(1, self.dpi));
        surface.paint_light(self.dpi, 0.0);
        Ok(surface)
    }

    fn paint(&self, dc: HDC) -> Result<()> {
        let canvas = self.typography.canvas(dc, self.width, self.height)?;
        canvas.clear(SURFACE);
        let pad = px(16, self.dpi);
        canvas.text(
            &self.title,
            Style::Title,
            INK,
            RECT {
                left: pad + px(18, self.dpi),
                top: px(11, self.dpi),
                right: self.width - px(88, self.dpi),
                bottom: px(35, self.dpi),
            },
            false,
        )?;
        canvas.text(
            "实时监控",
            Style::Body,
            MUTED,
            RECT {
                left: self.width - px(84, self.dpi),
                top: px(14, self.dpi),
                right: self.width - pad,
                bottom: px(35, self.dpi),
            },
            true,
        )?;
        canvas.fill(
            RECT {
                left: pad,
                top: px(43, self.dpi),
                right: self.width - pad,
                bottom: px(43, self.dpi) + 1,
            },
            BORDER,
        )?;
        let mut top = px(52, self.dpi);
        for row in &self.rows {
            let value_left = if row.label.is_empty() {
                pad
            } else {
                pad + self.label_width + px(18, self.dpi)
            };
            canvas.text(
                &row.label,
                Style::Body,
                MUTED,
                RECT {
                    left: pad,
                    top,
                    right: pad + self.label_width,
                    bottom: top + row.height,
                },
                false,
            )?;
            canvas.text(
                &row.value,
                Style::Body,
                INK,
                RECT {
                    left: value_left,
                    top,
                    right: self.width - pad,
                    bottom: top + row.height,
                },
                true,
            )?;
            top += row.height;
        }
        canvas.finish()
    }
}

fn position(work: RECT, anchor: RECT, width: i32, height: i32, gap: i32) -> (i32, i32) {
    let x = anchor.left + (anchor.right - anchor.left) / 2;
    let left = (x - width / 2).clamp(work.left, (work.right - width).max(work.left));
    let preferred_top = if anchor.top - gap - height >= work.top {
        anchor.top - gap - height
    } else {
        anchor.bottom + gap
    };
    let top = preferred_top.clamp(work.top, (work.bottom - height).max(work.top));
    (left, top)
}

fn popup_bounds(work: RECT, monitor: RECT, anchor: RECT) -> RECT {
    // Allow the transparent taskbar padding beside the readings. Clamping to
    // rcWork alone would reintroduce the gap even after anchoring to the grid.
    RECT {
        left: work.left.min(anchor.left).max(monitor.left),
        top: work.top.min(anchor.top).max(monitor.top),
        right: work.right.max(anchor.right).min(monitor.right),
        bottom: work.bottom.max(anchor.bottom).min(monitor.bottom),
    }
}

fn work_area(anchor: RECT) -> Result<RECT> {
    let x = anchor.left + (anchor.right - anchor.left) / 2;
    let y = anchor.top + (anchor.bottom - anchor.top) / 2;
    let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        GetMonitorInfoW(monitor, &mut info).ok()?;
    }
    Ok(popup_bounds(info.rcWork, info.rcMonitor, anchor))
}

pub fn show(owner: HWND, text: &str, anchor: RECT) -> Result<()> {
    let (mut raw, old_owner) = WINDOW.get();
    if raw == 0
        || old_owner != owner.0 as isize
        || !unsafe { IsWindow(HWND(raw as *mut _)).as_bool() }
    {
        destroy();
        raw = create(owner)?.0 as isize;
        WINDOW.set((raw, owner.0 as isize));
    }
    let hwnd = HWND(raw as *mut _);
    let dpi = unsafe { GetDpiForWindow(owner) }.max(96);
    let work = work_area(anchor)?;
    let card = Card::new(text, dpi, (work.right - work.left).min(px(480, dpi)))?;
    let (width, height) = (card.width, card.height);
    let (left, top) = position(work, anchor, width, height, px(2, dpi));
    let (started, was_animated) = FRAME.with(|frame| {
        frame
            .borrow()
            .as_ref()
            .map(|frame| (frame.started, frame.animated))
            .unwrap_or((Instant::now(), false))
    });
    let animated = animations_enabled();
    let mut surface = card.render()?;
    surface.paint_light(
        dpi,
        if animated {
            started.elapsed().as_secs_f64()
        } else {
            0.0
        },
    );
    surface.upload(hwnd, left, top)?;
    FRAME.with(|frame| {
        *frame.borrow_mut() = Some(PopupFrame {
            surface,
            dpi,
            left,
            top,
            started,
            animated,
        })
    });
    unsafe {
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            left,
            top,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOMOVE | SWP_NOSIZE,
        )?;
        if animated && !was_animated {
            SetTimer(hwnd, LIGHT_TIMER, 33, None);
        } else if !animated {
            let _ = KillTimer(hwnd, LIGHT_TIMER);
        }
    }
    Ok(())
}

pub fn hide() {
    let (raw, _) = WINDOW.get();
    if raw != 0 {
        clear_frame(HWND(raw as *mut _));
        unsafe {
            let _ = ShowWindow(HWND(raw as *mut _), SW_HIDE);
        }
    }
}

pub fn destroy() {
    let (raw, _) = WINDOW.replace((0, 0));
    if raw != 0 {
        clear_frame(HWND(raw as *mut _));
        unsafe {
            let _ = DestroyWindow(HWND(raw as *mut _));
        }
    }
}

fn create(owner: HWND) -> Result<HWND> {
    static CLASS: OnceLock<bool> = OnceLock::new();
    let instance = HINSTANCE(unsafe { GetModuleHandleW(None)? }.0);
    if !*CLASS.get_or_init(|| unsafe {
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: w!("SysPulseHoverCard"),
            ..Default::default()
        }) != 0
    }) {
        return Err(windows::core::Error::from_win32());
    }
    unsafe {
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED,
            w!("SysPulseHoverCard"),
            w!("SysPulse 实时监控"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            owner,
            HMENU::default(),
            instance,
            None,
        )?;
        Ok(hwnd)
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_TIMER if wparam.0 == LIGHT_TIMER => {
            if IsWindowVisible(hwnd).as_bool() {
                if let Err(error) = animate_light(hwnd) {
                    tracing::warn!(?error, "tooltip status animation stopped");
                }
            } else {
                clear_frame(hwnd);
            }
            LRESULT(0)
        }
        WM_SHOWWINDOW if wparam.0 == 0 => {
            clear_frame(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            clear_frame(hwnd);
            if WINDOW.get().0 == hwnd.0 as isize {
                WINDOW.set((0, 0));
            }
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            // The compositor retains the last uploaded frame.
            let _ = BeginPaint(hwnd, &mut ps);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point_anchor(x: i32, y: i32) -> RECT {
        RECT {
            left: x,
            right: x,
            top: y,
            bottom: y,
        }
    }

    #[test]
    fn positions_within_work_area_above_bottom_and_below_top_taskbars() {
        let work = RECT {
            left: -1920,
            top: 40,
            right: 0,
            bottom: 1040,
        };
        assert_eq!(
            position(work, point_anchor(-10, 1040), 360, 220, 10),
            (-360, 810)
        );
        assert_eq!(
            position(work, point_anchor(-1900, 0), 360, 220, 10),
            (-1920, 40)
        );
        assert_eq!(
            position(work, point_anchor(-960, 600), 360, 220, 10),
            (-1140, 370)
        );
    }

    #[test]
    fn taskbar_padding_does_not_force_a_gap_or_cover_top_docked_readings() {
        let monitor = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let bottom_work = RECT {
            bottom: 1032,
            ..monitor
        };
        let bottom_anchor = RECT {
            left: 1500,
            top: 1043,
            right: 1800,
            bottom: 1069,
        };
        let area = popup_bounds(bottom_work, monitor, bottom_anchor);
        let (_, top) = position(area, bottom_anchor, 320, 220, 2);
        assert_eq!(bottom_anchor.top - (top + 220), 2);
        assert!(top + 220 > bottom_work.bottom);

        let top_work = RECT { top: 48, ..monitor };
        let top_anchor = RECT {
            left: 1500,
            top: 11,
            right: 1800,
            bottom: 37,
        };
        let area = popup_bounds(top_work, monitor, top_anchor);
        let (_, top) = position(area, top_anchor, 320, 220, 2);
        assert_eq!(top - top_anchor.bottom, 2);
        assert!(top >= monitor.top && top + 220 <= monitor.bottom);
    }

    #[test]
    fn custom_card_preserves_full_values_at_multiple_dpi_scales() {
        let text = "SysPulse\r\n网速：↓107.8 KB/s  ↑1.20 MB/s\r\n硬盘：读 999.99 MB/s · 写 100.00 GB/s\r\nCPU 占用：100%\r\nCPU 温度：60°C\r\n内存：46%（14.76 GB / 31.85 GB）\r\n风扇：0 RPM";
        for dpi in [96, 120, 144, 192] {
            let card = Card::new(text, dpi, px(480, dpi)).unwrap();
            assert_eq!(card.rows.len(), 6);
            assert!(card.rows[1].value.ends_with("100.00 GB/s"));
            assert!(card.height > px(180, dpi));
            assert!(card.width <= px(480, dpi));
            for row in &card.rows {
                let available = card.width - px(32, dpi) - card.label_width - px(18, dpi);
                assert!(
                    card.typography
                        .measure(&row.value, Style::Body, 100_000)
                        .unwrap()
                        .0
                        <= available
                );
            }
            if dpi == 120 {
                if let Some(path) = std::env::var_os("SYSPULSE_TOOLTIP_PREVIEW") {
                    render_preview(&card, &path);
                }
            }
        }
    }

    #[test]
    fn popup_accepts_alpha_frames_without_activation_or_mouse_interception() {
        let hwnd = create(HWND::default()).unwrap();
        let style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32;
        assert_ne!(style & WS_EX_NOACTIVATE.0, 0);
        assert_ne!(style & WS_EX_TOOLWINDOW.0, 0);
        assert_ne!(style & WS_EX_LAYERED.0, 0);
        assert_ne!(style & WS_EX_TRANSPARENT.0, 0);
        assert_eq!(
            unsafe { SendMessageW(hwnd, WM_MOUSEACTIVATE, WPARAM(0), LPARAM(0)) }.0,
            MA_NOACTIVATE as isize
        );
        assert_eq!(
            unsafe { SendMessageW(hwnd, WM_NCHITTEST, WPARAM(0), LPARAM(0)) }.0,
            HTTRANSPARENT as isize
        );
        let card = Card::new("SysPulse\n硬盘：读 2.10 MB/s · 写 1.42 MB/s", 96, 480).unwrap();
        card.render().unwrap().upload(hwnd, 0, 0).unwrap();
        unsafe {
            assert!(!IsWindowVisible(hwnd).as_bool());
            // Updating the same window again must not fall back to opaque painting.
            card.render().unwrap().upload(hwnd, 0, 0).unwrap();
            DestroyWindow(hwnd).unwrap();
            assert!(!IsWindow(hwnd).as_bool());
        }
    }

    #[test]
    fn rounded_corners_have_symmetric_premultiplied_alpha_at_every_dpi() {
        for dpi in [96, 120, 144, 192] {
            let card = Card::new("SysPulse\n网速：↓112.0 KB/s ↑7.2 KB/s\nCPU 占用：11%\nCPU 温度：35°C\n内存：47%（15.03 GB / 31.85 GB）\n硬盘：读 0 B/s · 写 1.14 MB/s", dpi, px(480, dpi)).unwrap();
            let surface = card.render().unwrap();
            let pixels = surface.pixels();
            let alpha = |x: i32, y: i32| pixels[((y * card.width + x) * 4 + 3) as usize];
            assert_eq!(alpha(0, 0), 0);
            assert_eq!(alpha(card.width / 2, card.height / 2), 255);
            let mut partial = 0;
            for y in 0..px(10, dpi) {
                for x in 0..px(10, dpi) {
                    let a = alpha(x, y);
                    assert_eq!(a, alpha(card.width - 1 - x, y));
                    assert_eq!(a, alpha(x, card.height - 1 - y));
                    assert_eq!(a, alpha(card.width - 1 - x, card.height - 1 - y));
                    partial += usize::from(a > 0 && a < 255);
                }
            }
            assert!(
                partial >= 5,
                "DPI={dpi}: corner must have blended edge pixels"
            );
            assert!(pixels
                .chunks_exact(4)
                .all(|pixel| pixel[..3].iter().all(|&color| color <= pixel[3])));
            let (_, top) = position(
                RECT {
                    left: 0,
                    top: 0,
                    right: 1920,
                    bottom: 1040,
                },
                point_anchor(900, 1030),
                card.width,
                card.height,
                px(2, dpi),
            );
            assert_eq!(1030 - top - card.height, px(2, dpi));
            if dpi == 120 {
                if let Some(path) = std::env::var_os("SYSPULSE_TOOLTIP_SMOOTH_PREVIEW") {
                    render_preview(&card, &path);
                }
            }
        }
    }

    #[test]
    fn directwrite_text_has_smooth_grayscale_edges_without_cleartype_color_fringes() {
        for dpi in [96, 120, 144, 192] {
            let typography = Typography::new(dpi).unwrap();
            let surface = LayerSurface::new(px(300, dpi), px(44, dpi)).unwrap();
            let canvas = typography
                .canvas(surface.dc, surface.width, surface.height)
                .unwrap();
            canvas.clear(rgb(255, 255, 255));
            canvas
                .text(
                    "SysPulse 网速 内存 51% 48°C",
                    Style::Body,
                    rgb(0, 0, 0),
                    RECT {
                        left: px(4, dpi),
                        top: px(4, dpi),
                        right: surface.width,
                        bottom: surface.height,
                    },
                    false,
                )
                .unwrap();
            canvas.finish().unwrap();
            unsafe {
                let _ = GdiFlush();
            }
            let mut partial = 0;
            let mut ink = 0;
            for pixel in surface.pixels().chunks_exact(4) {
                assert_eq!(pixel[0], pixel[1], "DPI={dpi}: blue/green fringe");
                assert_eq!(pixel[1], pixel[2], "DPI={dpi}: green/red fringe");
                partial += usize::from(pixel[0] > 0 && pixel[0] < 255);
                ink += usize::from(pixel[0] < 80);
            }
            assert!(partial > 100, "DPI={dpi}: glyph edges must be antialiased");
            assert!(ink > 100, "DPI={dpi}: glyph strokes must remain dark");
        }
    }

    #[test]
    fn breathing_light_changes_only_its_patch_and_never_accumulates_blur() {
        for dpi in [96, 120, 144, 192] {
            let card =
                Card::new("SysPulse\nCPU 占用：40%\nCPU 温度：48°C", dpi, px(480, dpi)).unwrap();
            let mut surface = card.render().unwrap();
            let initial = surface.pixels().to_vec();
            surface.paint_light(dpi, 1.2);
            let mut changed = 0;
            for (i, (before, after)) in initial
                .chunks_exact(4)
                .zip(surface.pixels().chunks_exact(4))
                .enumerate()
            {
                if before != after {
                    let x = (i % card.width as usize) as i32;
                    let y = (i / card.width as usize) as i32;
                    assert!((x - px(20, dpi)).abs() <= px(10, dpi));
                    assert!((y - px(24, dpi)).abs() <= px(10, dpi));
                    changed += 1;
                }
            }
            assert!(changed > 100);
            surface.paint_light(dpi, 2.4);
            assert_eq!(initial, surface.pixels());
        }
    }

    #[test]
    fn hiding_or_destroying_popup_releases_animation_timer_and_frame() {
        let hwnd = create(HWND::default()).unwrap();
        WINDOW.set((hwnd.0 as isize, 0));
        let frame = || PopupFrame {
            surface: Card::new("SysPulse\nCPU：40%", 96, 480)
                .unwrap()
                .render()
                .unwrap(),
            dpi: 96,
            left: 0,
            top: 0,
            started: Instant::now(),
            animated: true,
        };
        FRAME.with(|state| *state.borrow_mut() = Some(frame()));
        unsafe {
            assert_ne!(SetTimer(hwnd, LIGHT_TIMER, 33, None), 0);
        }
        hide();
        FRAME.with(|state| assert!(state.borrow().is_none()));
        unsafe {
            assert!(KillTimer(hwnd, LIGHT_TIMER).is_err());
        }
        FRAME.with(|state| *state.borrow_mut() = Some(frame()));
        unsafe {
            assert_ne!(SetTimer(hwnd, LIGHT_TIMER, 33, None), 0);
        }
        destroy();
        FRAME.with(|state| assert!(state.borrow().is_none()));
        assert_eq!(WINDOW.get(), (0, 0));
        unsafe {
            assert!(!IsWindow(hwnd).as_bool());
        }
    }

    #[test]
    fn hd_card_preview_at_multiple_dpi_scales() {
        if let Some(directory) = std::env::var_os("SYSPULSE_TOOLTIP_HD_PREVIEW") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            for dpi in [96, 120, 144, 192] {
                let card = Card::new("SysPulse\n网速：↓120.0 KB/s  ↑3.3 KB/s\nCPU 占用：40%\nCPU 温度：48°C\n内存：51%（16.40 GB / 31.85 GB）\nCPU 频率：3.77 GHz", dpi, px(480, dpi)).unwrap();
                render_preview(
                    &card,
                    directory.join(format!("tooltip-{dpi}.bmp")).as_os_str(),
                );
                if dpi == 120 {
                    let mut surface = card.render().unwrap();
                    for frame in 0..24 {
                        surface.paint_light(dpi, frame as f64 * 0.1);
                        save_preview(
                            &surface,
                            directory.join(format!("pulse-{frame:02}.bmp")).as_os_str(),
                        );
                    }
                }
            }
        }
    }

    fn render_preview(card: &Card, path: &std::ffi::OsStr) {
        let surface = card.render().unwrap();
        save_preview(&surface, path);
    }

    fn save_preview(surface: &LayerSurface, path: &std::ffi::OsStr) {
        let mut pixels = surface.pixels().to_vec();
        // Composite the same premultiplied frame used by the real popup onto a
        // dark taskbar-like backdrop so halos and uneven corners are visible.
        for pixel in pixels.chunks_exact_mut(4) {
            let transparent = 255 - u32::from(pixel[3]);
            for (channel, background) in [70u32, 64, 24].into_iter().enumerate() {
                pixel[channel] =
                    (u32::from(pixel[channel]) + (background * transparent + 127) / 255) as u8;
            }
            pixel[3] = 255;
        }
        let info = BITMAPINFOHEADER {
            biSize: 40,
            biWidth: surface.width,
            biHeight: -surface.height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut output = Vec::with_capacity(54 + pixels.len());
        output.extend_from_slice(b"BM");
        output.extend_from_slice(&((54 + pixels.len()) as u32).to_le_bytes());
        output.extend_from_slice(&[0; 4]);
        output.extend_from_slice(&54u32.to_le_bytes());
        unsafe {
            output.extend_from_slice(std::slice::from_raw_parts(
                (&info as *const BITMAPINFOHEADER).cast(),
                40,
            ));
        }
        output.extend_from_slice(&pixels);
        std::fs::write(path, output).unwrap();
    }
}
