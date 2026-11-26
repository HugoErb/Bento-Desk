//! Drawing with Direct2D (software render target) and DirectWrite into a
//! 32-bit DIB, presented with `UpdateLayeredWindow` (per-pixel alpha).
//!
//! The software target avoids loading a Direct3D device and GPU driver in
//! the resident process; surfaces are redrawn only on events.
//! <https://learn.microsoft.com/windows/win32/direct2d/supported-pixel-formats-and-alpha-modes>
//! <https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-updatelayeredwindow>

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;

use bento_core::color::Rgba;
use bento_core::geometry::{Point, Rect};
use bento_core::image::Image;
use windows::Win32::Foundation::{COLORREF, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
    D2D1_BITMAP_PROPERTIES, D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING,
    DWRITE_TEXT_METRICS, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP, DWriteCreateFactory, IDWriteFactory,
    IDWriteFontFamily, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HBITMAP, HDC,
    HGDIOBJ, SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{ULW_ALPHA, UpdateLayeredWindow};
use windows_numerics::Vector2;

use crate::error::{Context, Result, WinError};
use crate::window::Hwnd;

/// Floating-point rectangle in pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectF {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl RectF {
    pub fn new(left: f32, top: f32, right: f32, bottom: f32) -> RectF {
        RectF {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn xywh(x: f32, y: f32, w: f32, h: f32) -> RectF {
        RectF::new(x, y, x + w, y + h)
    }

    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }

    pub fn inset(&self, d: f32) -> RectF {
        RectF::new(self.left + d, self.top + d, self.right - d, self.bottom - d)
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }

    fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F {
            left: self.left,
            top: self.top,
            right: self.right,
            bottom: self.bottom,
        }
    }
}

impl From<Rect> for RectF {
    fn from(r: Rect) -> RectF {
        RectF::new(r.left as f32, r.top as f32, r.right as f32, r.bottom as f32)
    }
}

fn color(c: Rgba) -> D2D1_COLOR_F {
    let [r, g, b, a] = c.to_f32();
    D2D1_COLOR_F { r, g, b, a }
}

/// Horizontal text alignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// Text appearance.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub family: String,
    /// Size in pixels.
    pub size: f32,
    pub bold: bool,
    pub color: Rgba,
    pub align: Align,
    /// Centered vertically (single line) or top aligned.
    pub vcenter: bool,
    pub wrap: bool,
}

impl TextStyle {
    pub fn new(family: &str, size: f32, color: Rgba) -> TextStyle {
        TextStyle {
            family: family.to_owned(),
            size,
            bold: false,
            color,
            align: Align::Left,
            vcenter: true,
            wrap: false,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct FormatKey {
    family: String,
    size_x10: u32,
    bold: bool,
    align: Align,
    vcenter: bool,
    wrap: bool,
}

/// A font family installed on the system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFamily {
    /// Family name, in English when the font provides it: the name
    /// DirectWrite resolves when it creates a text format.
    pub name: String,
    /// The glyphs are symbols, not letters (Wingdings, Marlett...).
    pub symbol: bool,
}

/// Font families of the system collection, sorted by name, without
/// duplicates.
pub fn system_font_families() -> Result<Vec<FontFamily>> {
    // SAFETY: shared DirectWrite factory.
    let factory: IDWriteFactory =
        unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }.ctx("DWriteCreateFactory")?;
    let mut collection = None;
    // SAFETY: `collection` is a valid out pointer for the call.
    unsafe { factory.GetSystemFontCollection(&mut collection, false) }
        .ctx("GetSystemFontCollection")?;
    let collection = collection.ok_or(WinError::NotFound("system font collection"))?;
    // SAFETY: query on a valid collection.
    let count = unsafe { collection.GetFontFamilyCount() };
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        // SAFETY: the index is below the family count.
        let Ok(family) = (unsafe { collection.GetFontFamily(i) }) else {
            continue;
        };
        let Some(name) = family_name(&family) else {
            continue;
        };
        // SAFETY: valid family; the arguments are valid enumeration values.
        let font = unsafe {
            family.GetFirstMatchingFont(
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
            )
        };
        // SAFETY: query on a valid font.
        let symbol = font.is_ok_and(|f| unsafe { f.IsSymbolFont() }.as_bool());
        out.push(FontFamily { name, symbol });
    }
    out.sort_by_cached_key(|f| f.name.to_lowercase());
    out.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    Ok(out)
}

/// English name of a family, or its first name.
fn family_name(family: &IDWriteFontFamily) -> Option<String> {
    // SAFETY: valid family.
    let names = unsafe { family.GetFamilyNames() }.ok()?;
    let mut index = 0;
    let mut exists = windows::core::BOOL(0);
    // SAFETY: valid out pointers; the locale is a static NUL-terminated string.
    unsafe { names.FindLocaleName(windows::core::w!("en-us"), &mut index, &mut exists) }.ok()?;
    if !exists.as_bool() {
        index = 0;
    }
    // SAFETY: the index is valid (found, or the first of a non-empty list).
    let len = unsafe { names.GetStringLength(index) }.ok()?;
    let mut buf = vec![0u16; len as usize + 1];
    // SAFETY: the buffer holds the string and its terminating NUL.
    unsafe { names.GetString(index, &mut buf) }.ok()?;
    let name = crate::util::from_wide(&buf);
    (!name.is_empty()).then_some(name)
}

/// Direct2D and DirectWrite factories plus caches. One per UI thread.
pub struct Gfx {
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    formats: RefCell<HashMap<FormatKey, IDWriteTextFormat>>,
}

impl std::fmt::Debug for Gfx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Gfx")
    }
}

impl Gfx {
    pub fn new() -> Result<Gfx> {
        // SAFETY: single-threaded factory used only on this thread.
        // https://learn.microsoft.com/windows/win32/api/d2d1/nf-d2d1-d2d1createfactory
        let d2d: ID2D1Factory =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) }
                .ctx("D2D1CreateFactory")?;
        // SAFETY: shared DirectWrite factory.
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }
            .ctx("DWriteCreateFactory")?;
        Ok(Gfx {
            d2d,
            dwrite,
            formats: RefCell::new(HashMap::new()),
        })
    }

    fn format(&self, s: &TextStyle) -> Result<IDWriteTextFormat> {
        let key = FormatKey {
            family: s.family.clone(),
            size_x10: (s.size * 10.0).round().max(1.0) as u32,
            bold: s.bold,
            align: s.align,
            vcenter: s.vcenter,
            wrap: s.wrap,
        };
        if let Some(f) = self.formats.borrow().get(&key) {
            return Ok(f.clone());
        }
        let fam = crate::util::wide(&s.family);
        let locale = crate::util::wide("");
        let weight = if s.bold {
            DWRITE_FONT_WEIGHT_SEMI_BOLD
        } else {
            DWRITE_FONT_WEIGHT_NORMAL
        };
        // SAFETY: buffers outlive the call; a missing family falls back to
        // the system font.
        let f = unsafe {
            self.dwrite.CreateTextFormat(
                crate::util::pcwstr(&fam),
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                s.size.max(1.0),
                crate::util::pcwstr(&locale),
            )
        }
        .ctx("CreateTextFormat")?;
        // SAFETY: plain setters on a format we own.
        unsafe {
            let _ = f.SetTextAlignment(match s.align {
                Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            let _ = f.SetParagraphAlignment(if s.vcenter {
                DWRITE_PARAGRAPH_ALIGNMENT_CENTER
            } else {
                DWRITE_PARAGRAPH_ALIGNMENT_NEAR
            });
            let _ = f.SetWordWrapping(if s.wrap {
                DWRITE_WORD_WRAPPING_WRAP
            } else {
                DWRITE_WORD_WRAPPING_NO_WRAP
            });
            if let Ok(sign) = self.dwrite.CreateEllipsisTrimmingSign(&f) {
                let trimming = DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    delimiter: 0,
                    delimiterCount: 0,
                };
                let _ = f.SetTrimming(&trimming, &sign);
            }
        }
        self.formats.borrow_mut().insert(key, f.clone());
        Ok(f)
    }

    /// Width and height of a text laid out within `max_width`.
    pub fn measure(&self, text: &str, style: &TextStyle, max_width: f32) -> (f32, f32) {
        let Ok(format) = self.format(style) else {
            return (0.0, 0.0);
        };
        let w: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: the buffer outlives the call.
        let layout = unsafe {
            self.dwrite
                .CreateTextLayout(&w, &format, max_width.max(1.0), 10_000.0)
        };
        let Ok(layout) = layout else {
            return (0.0, 0.0);
        };
        let mut m = DWRITE_TEXT_METRICS::default();
        // SAFETY: valid out pointer.
        if unsafe { layout.GetMetrics(&mut m) }.is_err() {
            return (0.0, 0.0);
        }
        (m.widthIncludingTrailingWhitespace, m.height)
    }
}

/// Off-screen 32-bit premultiplied surface bound to a layered window.
pub struct Surface {
    hdc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    width: i32,
    height: i32,
    /// Rows allocated in the bitmap (at least `height`).
    capacity: i32,
    rt: ID2D1DCRenderTarget,
}

impl std::fmt::Debug for Surface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Surface({}x{})", self.width, self.height)
    }
}

impl Surface {
    pub fn new(gfx: &Gfx, width: i32, height: i32) -> Result<Surface> {
        Surface::with_capacity(gfx, width, height, height)
    }

    /// Surface of `width` x `height` whose bitmap holds `capacity` rows, so
    /// that [`Surface::reuse`] can later make it taller without allocating.
    pub fn with_capacity(gfx: &Gfx, width: i32, height: i32, capacity: i32) -> Result<Surface> {
        let width = width.max(1);
        let height = height.max(1);
        let capacity = capacity.max(height);
        // SAFETY: memory DC compatible with the screen.
        let hdc = unsafe { CreateCompatibleDC(None) };
        if hdc.is_invalid() {
            return Err(WinError::Other("CreateCompatibleDC failed".into()));
        }
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -capacity, // top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        // SAFETY: `bmi` describes a 32-bit top-down DIB; `bits` receives
        // the pixel pointer owned by the bitmap.
        // https://learn.microsoft.com/windows/win32/api/wingdi/nf-wingdi-createdibsection
        let bitmap = match unsafe {
            CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
        } {
            Ok(b) => b,
            Err(e) => {
                // SAFETY: DC created above.
                let _ = unsafe { DeleteDC(hdc) };
                return Err(WinError::Api {
                    context: "CreateDIBSection",
                    source: e,
                });
            }
        };
        // SAFETY: select our bitmap into our DC; the old object is restored
        // in Drop.
        let old = unsafe { SelectObject(hdc, HGDIOBJ(bitmap.0)) };
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        // SAFETY: valid properties.
        let rt = unsafe { gfx.d2d.CreateDCRenderTarget(&props) };
        let rt = match rt {
            Ok(rt) => rt,
            Err(e) => {
                // SAFETY: undo the selection and free the GDI objects.
                unsafe {
                    SelectObject(hdc, old);
                    let _ = DeleteObject(HGDIOBJ(bitmap.0));
                    let _ = DeleteDC(hdc);
                }
                return Err(WinError::Api {
                    context: "CreateDCRenderTarget",
                    source: e,
                });
            }
        };
        Ok(Surface {
            hdc,
            bitmap,
            old,
            bits: bits as *mut u8,
            width,
            height,
            capacity,
            rt,
        })
    }

    pub fn size(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    /// Takes the size `width` x `height` without allocating when the width
    /// is unchanged and the bitmap has enough rows. Only the first `height`
    /// rows are drawn and presented. Returns false when a new surface is
    /// needed.
    pub fn reuse(&mut self, width: i32, height: i32) -> bool {
        let (width, height) = (width.max(1), height.max(1));
        if width != self.width || height > self.capacity {
            return false;
        }
        self.height = height;
        true
    }

    /// Clears to transparent and runs the drawing closure.
    pub fn draw(&mut self, gfx: &Gfx, f: impl FnOnce(&mut Canvas<'_>)) -> Result<()> {
        let rc = RECT {
            left: 0,
            top: 0,
            right: self.width,
            bottom: self.height,
        };
        // SAFETY: DC and render target belong to this surface.
        unsafe {
            self.rt.BindDC(self.hdc, &rc).ctx("BindDC")?;
            self.rt.BeginDraw();
            self.rt
                .SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            self.rt.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            self.rt.Clear(Some(&color(Rgba::TRANSPARENT)));
        }
        let mut canvas = Canvas { rt: &self.rt, gfx };
        f(&mut canvas);
        // SAFETY: matching EndDraw.
        unsafe { self.rt.EndDraw(None, None) }.ctx("EndDraw")
    }

    /// Copies the surface to a layered window (position unchanged, size
    /// set to the surface size).
    pub fn present(&self, hwnd: Hwnd) -> Result<()> {
        let size = SIZE {
            cx: self.width,
            cy: self.height,
        };
        let src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        // SAFETY: the window has WS_EX_LAYERED; the DC holds a 32-bit
        // premultiplied bitmap of `size`.
        unsafe {
            UpdateLayeredWindow(
                hwnd.raw(),
                None,
                None,
                Some(&size),
                Some(self.hdc),
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        }
        .ctx("UpdateLayeredWindow")
    }

    /// Copy of the pixels (premultiplied BGRA).
    pub fn snapshot(&self) -> Image {
        let len = (self.width as usize) * (self.height as usize) * 4;
        // SAFETY: `bits` points to the DIB of `len` bytes owned by
        // `self.bitmap`, alive as long as `self`; no draw is in progress.
        let px = unsafe { std::slice::from_raw_parts(self.bits, len) }.to_vec();
        Image {
            width: self.width as u32,
            height: self.height as u32,
            pixels: px,
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: restore the original object, then free what we created.
        unsafe {
            SelectObject(self.hdc, self.old);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = DeleteDC(self.hdc);
        }
    }
}

/// Drawing operations on a surface during `Surface::draw`.
pub struct Canvas<'a> {
    rt: &'a ID2D1DCRenderTarget,
    gfx: &'a Gfx,
}

impl Canvas<'_> {
    pub fn gfx(&self) -> &Gfx {
        self.gfx
    }

    fn brush(&self, c: Rgba) -> Option<windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush> {
        // SAFETY: valid color pointer.
        unsafe { self.rt.CreateSolidColorBrush(&color(c), None) }.ok()
    }

    pub fn fill_rect(&mut self, r: RectF, c: Rgba) {
        if let Some(b) = self.brush(c) {
            // SAFETY: valid rect and brush.
            unsafe { self.rt.FillRectangle(&r.d2d(), &b) };
        }
    }

    pub fn fill_round_rect(&mut self, r: RectF, radius: f32, c: Rgba) {
        if let Some(b) = self.brush(c) {
            let rr = D2D1_ROUNDED_RECT {
                rect: r.d2d(),
                radiusX: radius,
                radiusY: radius,
            };
            // SAFETY: valid rounded rect and brush.
            unsafe { self.rt.FillRoundedRectangle(&rr, &b) };
        }
    }

    pub fn stroke_round_rect(&mut self, r: RectF, radius: f32, c: Rgba, width: f32) {
        if let Some(b) = self.brush(c) {
            let rr = D2D1_ROUNDED_RECT {
                rect: r.d2d(),
                radiusX: radius,
                radiusY: radius,
            };
            // SAFETY: valid rounded rect and brush, default stroke style.
            unsafe { self.rt.DrawRoundedRectangle(&rr, &b, width, None) };
        }
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, c: Rgba) {
        if let Some(b) = self.brush(c) {
            let e = D2D1_ELLIPSE {
                point: Vector2 { X: cx, Y: cy },
                radiusX: radius,
                radiusY: radius,
            };
            // SAFETY: valid ellipse and brush.
            unsafe { self.rt.FillEllipse(&e, &b) };
        }
    }

    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgba, width: f32) {
        if let Some(b) = self.brush(c) {
            // SAFETY: valid brush, default stroke style.
            unsafe {
                self.rt.DrawLine(
                    Vector2 { X: x0, Y: y0 },
                    Vector2 { X: x1, Y: y1 },
                    &b,
                    width,
                    None,
                )
            };
        }
    }

    pub fn text(&mut self, text: &str, r: RectF, style: &TextStyle) {
        let Ok(format) = self.gfx.format(style) else {
            return;
        };
        let Some(b) = self.brush(style.color) else {
            return;
        };
        let w: Vec<u16> = text.encode_utf16().collect();
        let opts = if style.wrap {
            D2D1_DRAW_TEXT_OPTIONS_CLIP
        } else {
            D2D1_DRAW_TEXT_OPTIONS_NONE
        };
        // SAFETY: valid buffer, format, rect and brush.
        unsafe {
            self.rt.DrawText(
                &w,
                &format,
                &r.d2d(),
                &b,
                opts,
                DWRITE_MEASURING_MODE_NATURAL,
            )
        };
    }

    /// Draws a premultiplied BGRA image scaled into `dest`.
    pub fn image(&mut self, img: &Image, dest: RectF, opacity: f32) {
        if img.is_empty() {
            return;
        }
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        // SAFETY: `img.pixels` holds width*height*4 bytes with a pitch of
        // width*4; D2D copies the data.
        let bmp = unsafe {
            self.rt.CreateBitmap(
                D2D_SIZE_U {
                    width: img.width,
                    height: img.height,
                },
                Some(img.pixels.as_ptr() as *const c_void),
                img.width * 4,
                &props,
            )
        };
        if let Ok(bmp) = bmp {
            // SAFETY: valid bitmap and rect.
            unsafe {
                self.rt.DrawBitmap(
                    &bmp,
                    Some(&dest.d2d()),
                    opacity.clamp(0.0, 1.0),
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                )
            };
        }
    }

    /// Fills a rounded rectangle with an image stretched over `image_rect`
    /// (keeps rounded corners, unlike [`Canvas::image`]).
    pub fn fill_round_rect_image(
        &mut self,
        r: RectF,
        radius: f32,
        img: &Image,
        image_rect: RectF,
        opacity: f32,
    ) {
        if img.is_empty() {
            return;
        }
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        // SAFETY: as in `image`.
        let Ok(bmp) = (unsafe {
            self.rt.CreateBitmap(
                D2D_SIZE_U {
                    width: img.width,
                    height: img.height,
                },
                Some(img.pixels.as_ptr() as *const c_void),
                img.width * 4,
                &props,
            )
        }) else {
            return;
        };
        // SAFETY: valid bitmap; default brush properties.
        let Ok(brush) = (unsafe { self.rt.CreateBitmapBrush(&bmp, None, None) }) else {
            return;
        };
        let sx = image_rect.width() / img.width as f32;
        let sy = image_rect.height() / img.height as f32;
        let m = windows_numerics::Matrix3x2 {
            M11: sx,
            M12: 0.0,
            M21: 0.0,
            M22: sy,
            M31: image_rect.left,
            M32: image_rect.top,
        };
        let rr = D2D1_ROUNDED_RECT {
            rect: r.d2d(),
            radiusX: radius,
            radiusY: radius,
        };
        // SAFETY: valid brush, matrix and geometry.
        unsafe {
            brush.SetTransform(&m);
            brush.SetOpacity(opacity.clamp(0.0, 1.0));
            self.rt.FillRoundedRectangle(&rr, &brush);
        }
    }

    /// Restricts drawing to `r` until [`Canvas::pop_clip`].
    pub fn push_clip(&mut self, r: RectF) {
        // SAFETY: valid rect; balanced by pop_clip.
        unsafe {
            self.rt
                .PushAxisAlignedClip(&r.d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE)
        };
    }

    pub fn pop_clip(&mut self) {
        // SAFETY: balanced with push_clip.
        unsafe { self.rt.PopAxisAlignedClip() };
    }
}

/// Converts a screen point into surface coordinates of a window at `origin`.
pub fn local(origin: Point, p: Point) -> (f32, f32) {
    ((p.x - origin.x) as f32, (p.y - origin.y) as f32)
}
