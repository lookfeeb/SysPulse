//! DirectWrite layouts measured and drawn at the popup's physical pixel size.
use super::px;
use windows::core::{w, Result};
use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Direct2D::{Common::*, *};
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HDC;

#[derive(Clone, Copy)]
pub(super) enum Style {
    Body,
    Title,
}

pub(super) struct Typography {
    write: IDWriteFactory,
    draw: ID2D1Factory,
    body: IDWriteTextFormat,
    title: IDWriteTextFormat,
}

impl Typography {
    pub fn new(dpi: u32) -> Result<Self> {
        unsafe {
            let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let draw = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let format = |size, weight| {
                write.CreateTextFormat(
                    w!("Microsoft YaHei UI"),
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    px(size, dpi) as f32,
                    w!("zh-CN"),
                )
            };
            let body = format(13, DWRITE_FONT_WEIGHT_NORMAL)?;
            let title = format(15, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
            Ok(Self {
                write,
                draw,
                body,
                title,
            })
        }
    }

    fn layout(&self, text: &str, style: Style, width: i32) -> Result<IDWriteTextLayout> {
        let format = match style {
            Style::Body => &self.body,
            Style::Title => &self.title,
        };
        unsafe {
            self.write.CreateTextLayout(
                &text.encode_utf16().collect::<Vec<_>>(),
                format,
                width.max(1) as f32,
                100_000.0,
            )
        }
    }

    pub fn measure(&self, text: &str, style: Style, width: i32) -> Result<(i32, i32)> {
        let layout = self.layout(text, style, width)?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe {
            layout.GetMetrics(&mut metrics)?;
        }
        Ok((
            metrics.widthIncludingTrailingWhitespace.ceil() as i32,
            metrics.height.ceil() as i32,
        ))
    }

    pub fn canvas(&self, dc: HDC, width: i32, height: i32) -> Result<Canvas<'_>> {
        unsafe {
            let target = self
                .draw
                .CreateDCRenderTarget(&D2D1_RENDER_TARGET_PROPERTIES {
                    r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_IGNORE,
                    },
                    // Card geometry and font sizes are already in physical pixels.
                    // Keeping the DC target at 96 DPI prevents a second scaling pass.
                    dpiX: 96.0,
                    dpiY: 96.0,
                    usage: D2D1_RENDER_TARGET_USAGE_NONE,
                    minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
                })?;
            target.BindDC(
                dc,
                &RECT {
                    right: width,
                    bottom: height,
                    ..Default::default()
                },
            )?;
            // Subpixel ClearType produces colored fringes on a layered surface.
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            target.BeginDraw();
            Ok(Canvas {
                typography: self,
                target,
            })
        }
    }
}

pub(super) struct Canvas<'a> {
    typography: &'a Typography,
    target: ID2D1DCRenderTarget,
}

fn color(value: COLORREF) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: (value.0 & 255) as f32 / 255.0,
        g: ((value.0 >> 8) & 255) as f32 / 255.0,
        b: ((value.0 >> 16) & 255) as f32 / 255.0,
        a: 1.0,
    }
}

impl Canvas<'_> {
    pub fn clear(&self, background: COLORREF) {
        unsafe {
            self.target.Clear(Some(&color(background)));
        }
    }

    pub fn fill(&self, rect: RECT, value: COLORREF) -> Result<()> {
        unsafe {
            let brush = self.target.CreateSolidColorBrush(&color(value), None)?;
            self.target.FillRectangle(
                &D2D_RECT_F {
                    left: rect.left as f32,
                    top: rect.top as f32,
                    right: rect.right as f32,
                    bottom: rect.bottom as f32,
                },
                &brush,
            );
        }
        Ok(())
    }

    pub fn text(
        &self,
        text: &str,
        style: Style,
        value: COLORREF,
        rect: RECT,
        right: bool,
    ) -> Result<()> {
        let layout = self
            .typography
            .layout(text, style, rect.right - rect.left)?;
        unsafe {
            if right {
                layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
            }
            let brush = self.target.CreateSolidColorBrush(&color(value), None)?;
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: rect.left as f32,
                    y: rect.top as f32,
                },
                &layout,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
        Ok(())
    }

    pub fn finish(self) -> Result<()> {
        unsafe { self.target.EndDraw(None, None) }
    }
}
